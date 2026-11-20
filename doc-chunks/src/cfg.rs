//! Symbolic configuration predicates used to project conditional documentation.
//!
//! Projections select source fragments for one documentation owner, including
//! unexpanded macros. `OwnerDocumentation::select_complete_literals` checks text
//! completeness later, before a `LiteralSet` can be built.
//!
//! We deliberately do not evaluate the host's configuration: documentation for
//! other targets and disabled features needs spellchecking as well.
//!
//! Parsing accepts cfg spellings from supported Rust editions and releases,
//! including feature-gated version and compact-target predicates. Identifier
//! normalization follows Rust; configuration values remain ordinary strings.

use biodivine_lib_bdd::{op_function, Bdd, BddVariable, BddVariableSet};
use indexmap::IndexSet;
use std::collections::HashSet;
use syn::ext::IdentExt;
use syn::punctuated::Punctuated;
use syn::{parenthesized, Ident, LitBool, LitStr, Token};
use unicode_normalization::UnicodeNormalization;

#[cfg(test)]
mod tests;

/// An atomic cfg option or compiler-version test used as one symbolic variable.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Atom {
    Flag(String),
    Value(String, String),
    /// A feature-gated compiler-version test, kept symbolic rather than host-evaluated.
    Version(RustVersion),
}

/// A validated Rust release threshold, ordered numerically rather than as a string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct RustVersion([u16; 3]);

impl RustVersion {
    /// Match rustc 1.88's attribute version parser, including its optional zero patch.
    fn parse(literal: &LitStr) -> syn::Result<Self> {
        // rustc_attr_parsing::attributes::util::parse_version uses u16 components,
        // rejects '-' suffixes, and otherwise delegates component acceptance to FromStr.
        let value = literal.value();
        let mut components = value.splitn(3, '.');
        let release = (|| {
            if value.contains('-') {
                return None;
            }
            let major = components.next()?.parse().ok()?;
            let minor = components.next()?.parse().ok()?;
            let patch = components.next().unwrap_or("0").parse().ok()?;
            Some(Self([major, minor, patch]))
        })();
        release.ok_or_else(|| {
            syn::Error::new_spanned(
                literal,
                "expected a Rust release version with two or three u16 components",
            )
        })
    }

    /// Retain the existing short spelling for zero patches while canonicalizing equivalents.
    fn as_predicate(self) -> String {
        let Self([major, minor, patch]) = self;
        let version = if patch == 0 {
            format!("{major}.{minor}")
        } else {
            format!("{major}.{minor}.{patch}")
        };
        format!("version({version:?})")
    }
}

/// Maximum nested cfg operators accepted before recursive parsing and AST traversal become unsafe.
const MAX_PREDICATE_DEPTH: usize = 32;

/// A parsed cfg expression retaining its Boolean operators before BDD reduction.
#[derive(Clone, Debug)]
pub(super) enum Predicate {
    Constant(bool),
    Atom(Atom),
    Not(Box<Predicate>),
    All(Vec<Predicate>),
    Any(Vec<Predicate>),
}

impl syn::parse::Parse for Predicate {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        Self::parse_bounded(input, 0)
    }
}

impl Predicate {
    /// Bound accepted AST depth so parsing, cloning, evaluation, compilation, and drop stay shallow.
    fn parse_bounded(input: syn::parse::ParseStream<'_>, depth: usize) -> syn::Result<Self> {
        if input.peek(LitBool) {
            return Ok(Self::Constant(input.parse::<LitBool>()?.value));
        }
        let ident = parse_cfg_identifier(input)?;
        let name = normalized_cfg_name(&ident);
        if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            return Ok(Self::Atom(Atom::Value(
                name,
                parse_cfg_string(input)?.value(),
            )));
        }
        if !input.peek(syn::token::Paren) {
            return Ok(Self::Atom(Atom::Flag(name)));
        }
        if depth >= MAX_PREDICATE_DEPTH && matches!(name.as_str(), "all" | "any" | "not" | "target")
        {
            return Err(input.error(format!(
                "cfg predicate nesting limit exceeded ({MAX_PREDICATE_DEPTH})"
            )));
        }
        let content;
        parenthesized!(content in input);
        match name.as_str() {
            "all" | "any" | "not" => {
                let mut predicates = Vec::new();
                while !content.is_empty() {
                    predicates.push(Self::parse_bounded(&content, depth + 1)?);
                    if content.is_empty() {
                        break;
                    }
                    content.parse::<Token![,]>()?;
                }
                match name.as_str() {
                    "all" => Ok(Self::All(predicates)),
                    "any" => Ok(Self::Any(predicates)),
                    _ if predicates.len() == 1 => {
                        Ok(Self::Not(Box::new(predicates.into_iter().next().unwrap())))
                    }
                    _ => Err(syn::Error::new_spanned(
                        ident,
                        "not() requires exactly one predicate",
                    )),
                }
            }
            "version" => {
                let version = parse_cfg_string(&content)?;
                content.parse::<Option<Token![,]>>()?;
                if !content.is_empty() {
                    return Err(content.error("version() requires one string literal"));
                }
                Ok(Self::Atom(Atom::Version(RustVersion::parse(&version)?)))
            }
            "target" => {
                // Compact target options are aliases, so they share atoms with target_* keys.
                let options = Punctuated::<TargetOption, Token![,]>::parse_terminated(&content)?;
                Ok(Self::All(Vec::from_iter(options.into_iter().map(
                    |option| {
                        Self::Atom(Atom::Value(format!("target_{}", option.key), option.value))
                    },
                ))))
            }
            _ => Err(syn::Error::new_spanned(ident, "unsupported cfg predicate")),
        }
    }
}

/// Parse cfg names across supported editions, including identifiers legal only before 2018.
fn parse_cfg_identifier(input: syn::parse::ParseStream<'_>) -> syn::Result<Ident> {
    if input.peek(Ident) {
        return input.parse::<Ident>();
    }
    let ident = input.call(Ident::parse_any)?;
    let legacy = [
        "async", "await", "dyn", "try", "gen", "self", "super", "Self", "crate",
    ]
    .into_iter()
    .any(|name| ident == name);
    if !legacy {
        return Err(syn::Error::new_spanned(ident, "expected a cfg identifier"));
    }
    Ok(ident)
}

/// Rust identifiers use NFC normalization; cfg string values must not be normalized.
fn normalized_cfg_name(ident: &Ident) -> String {
    String::from_iter(ident.unraw().to_string().nfc())
}

/// Cfg option values and version tests require unsuffixed string literals.
fn parse_cfg_string(input: syn::parse::ParseStream<'_>) -> syn::Result<LitStr> {
    let value = input.parse::<LitStr>()?;
    if !value.suffix().is_empty() {
        return Err(syn::Error::new_spanned(
            value,
            "cfg strings cannot have suffixes",
        ));
    }
    Ok(value)
}

/// One key-value entry inside feature-gated compact target syntax.
struct TargetOption {
    key: String,
    value: String,
}

impl syn::parse::Parse for TargetOption {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let key = normalized_cfg_name(&parse_cfg_identifier(input)?);
        input.parse::<Token![=]>()?;
        let value = parse_cfg_string(input)?.value();
        Ok(Self { key, value })
    }
}

impl Atom {
    fn as_predicate(&self) -> String {
        let (name, value) = match self {
            Self::Flag(name) => (name, None),
            Self::Value(name, value) => (name, Some(value)),
            Self::Version(version) => return version.as_predicate(),
        };
        // Rust 1.88 accepts these legacy names but forbids their raw-identifier spellings.
        let legacy = matches!(name.as_str(), "self" | "super" | "Self" | "crate");
        let name = if legacy || (syn::parse_str::<Ident>(name).is_ok() && name != "gen") {
            name.clone()
        } else {
            format!("r#{name}")
        };
        match value {
            Some(value) => format!("{name} = {value:?}"),
            None => name,
        }
    }
}

impl Predicate {
    pub(super) fn always() -> Self {
        Self::Constant(true)
    }

    /// Describe one complete assignment over the atoms in the projection scope.
    fn example<'a>(atoms: impl IntoIterator<Item = &'a Atom>, assignment: &[bool]) -> String {
        let mut terms = Vec::from_iter(atoms.into_iter().enumerate().map(|(index, atom)| {
            let predicate = atom.as_predicate();
            if assignment[index] {
                predicate
            } else {
                format!("not({predicate})")
            }
        }));
        match terms.len() {
            0 => "all()".to_owned(),
            1 => terms.pop().unwrap(),
            _ => format!("all({})", terms.join(", ")),
        }
    }

    fn collect_atoms(&self, atoms: &mut Vec<Atom>, limit: usize) -> Result<(), ProjectionError> {
        match self {
            Self::Constant(_) => {}
            Self::Atom(atom) => {
                if !atoms.contains(atom) {
                    if atoms.len() >= limit {
                        return Err(ProjectionError::Variables { limit });
                    }
                    atoms.push(atom.clone());
                }
            }
            Self::Not(predicate) => predicate.collect_atoms(atoms, limit)?,
            Self::All(predicates) | Self::Any(predicates) => {
                for predicate in predicates {
                    predicate.collect_atoms(atoms, limit)?;
                }
            }
        }
        Ok(())
    }

    /// Evaluate a concrete configuration without constructing a decision diagram.
    fn evaluate(&self, value: &impl Fn(&Atom) -> bool) -> bool {
        match self {
            Self::Constant(value) => *value,
            Self::Atom(atom) => value(atom),
            Self::Not(predicate) => !predicate.evaluate(value),
            Self::All(predicates) => predicates.iter().all(|predicate| predicate.evaluate(value)),
            Self::Any(predicates) => predicates.iter().any(|predicate| predicate.evaluate(value)),
        }
    }

    /// Visit every atom, including atoms in branches a concrete evaluation would skip.
    fn visit_atoms(&self, visit: &mut impl FnMut(&Atom)) {
        match self {
            Self::Constant(_) => {}
            Self::Atom(atom) => visit(atom),
            Self::Not(predicate) => predicate.visit_atoms(visit),
            Self::All(predicates) | Self::Any(predicates) => {
                for predicate in predicates {
                    predicate.visit_atoms(visit);
                }
            }
        }
    }

    // An independent truth-table oracle for checking the BDD translation.
    #[cfg(test)]
    fn matches(&self, atoms: &[Atom], assignment: usize) -> bool {
        match self {
            Self::Constant(value) => *value,
            Self::Atom(atom) => {
                let index = atoms
                    .iter()
                    .position(|candidate| candidate == atom)
                    .unwrap();
                assignment & (1 << index) != 0
            }
            Self::Not(predicate) => !predicate.matches(atoms, assignment),
            Self::All(predicates) => predicates
                .iter()
                .all(|predicate| predicate.matches(atoms, assignment)),
            Self::Any(predicates) => predicates
                .iter()
                .any(|predicate| predicate.matches(atoms, assignment)),
        }
    }
}

/// Per-owner projection limits for symbolic cfg reasoning and fallback configuration sampling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CfgBudget {
    /// Maximum symbolic variables, capped by the BDD library's representation limit.
    pub variables: usize,
    /// Maximum nodes in a BDD produced by a Boolean operation.
    pub bdd_nodes: usize,
    /// Maximum tasks explored by a Boolean operation.
    pub bdd_tasks: usize,
    /// Maximum symbolic regions, including the empty documentation selection.
    pub variants: usize,
    /// Maximum complete assignments sampled after overflow; zero disables sampling.
    pub samples: usize,
}

impl Default for CfgBudget {
    fn default() -> Self {
        Self {
            variables: 1024,
            bdd_nodes: 4096,
            bdd_tasks: 16384,
            variants: 1024,
            samples: 1024,
        }
    }
}

impl CfgBudget {
    fn variable_limit(self) -> usize {
        self.variables.min(usize::from(u16::MAX) - 2)
    }
}

/// A complexity limit that reduces configuration coverage to a bounded sample.
#[derive(Debug, thiserror::Error)]
pub(super) enum ProjectionError {
    #[error("cfg variable limit exceeded ({limit})")]
    Variables { limit: usize },
    #[error("cfg reasoning exceeded its BDD node or operation budget")]
    BddBudget,
    #[error("documentation selection limit exceeded ({limit})")]
    Variants { limit: usize },
}

/// A distinct selection of source-fragment indices and one configuration producing it.
///
/// Selected fragments may include unexpanded macros, so this does not guarantee complete text.
pub(super) struct Projection {
    pub(super) selected: Vec<usize>,
    pub(super) example: String,
}

/// The predicate whose processing first exceeded the symbolic coverage budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ProjectionSite {
    Guard(usize),
    Constraint(usize),
}

/// An incomplete-coverage warning located at a fragment guard or owner constraint.
#[derive(Debug)]
pub(super) struct ProjectionWarning {
    pub(super) error: ProjectionError,
    pub(super) site: ProjectionSite,
}

/// Fragment selections with cfg witnesses and a warning when configuration coverage was sampled.
pub(super) struct ProjectionReport {
    pub(super) projections: Vec<Projection>,
    pub(super) warning: Option<ProjectionWarning>,
}

/// Maps one owner's cfg atoms to a shared BDD variable order and enforces its projection budget.
struct ConditionSpace {
    atoms: Vec<Atom>,
    variables: BddVariableSet,
    budget: CfgBudget,
}

impl ConditionSpace {
    #[cfg(test)]
    fn new<'a>(predicates: impl Iterator<Item = &'a Predicate>) -> Result<Self, ProjectionError> {
        Self::with_budget(predicates, CfgBudget::default())
    }

    #[cfg(test)]
    fn with_budget<'a>(
        predicates: impl Iterator<Item = &'a Predicate>,
        budget: CfgBudget,
    ) -> Result<Self, ProjectionError> {
        let mut atoms = Vec::new();
        for predicate in predicates {
            predicate.collect_atoms(&mut atoms, budget.variable_limit())?;
        }
        // Cfg atoms can contain arbitrary Rust strings. Anonymous BDD variables
        // avoid imposing the library's variable-name syntax on those strings.
        let variables = BddVariableSet::new_anonymous(atoms.len() as u16);
        Ok(Self {
            atoms,
            variables,
            budget,
        })
    }

    fn compile(&self, predicate: &Predicate) -> Result<Bdd, ProjectionError> {
        match predicate {
            Predicate::Constant(true) => Ok(self.variables.mk_true()),
            Predicate::Constant(false) => Ok(self.variables.mk_false()),
            Predicate::Atom(atom) => {
                let index = self
                    .atoms
                    .iter()
                    .position(|candidate| candidate == atom)
                    .unwrap();
                Ok(self.variables.mk_var(BddVariable::from_index(index)))
            }
            Predicate::Not(inner) => Ok(self.compile(inner)?.into_not()),
            Predicate::All(predicates) => {
                let mut result = self.variables.mk_true();
                for inner in predicates {
                    // A contradiction determines the conjunction without compiling later terms.
                    if result.is_false() {
                        break;
                    }
                    result =
                        self.budget
                            .combine(&result, &self.compile(inner)?, op_function::and)?;
                }
                Ok(result)
            }
            Predicate::Any(predicates) => {
                let mut result = self.variables.mk_false();
                for inner in predicates {
                    // A tautology determines the disjunction without compiling later terms.
                    if result.is_true() {
                        break;
                    }
                    result =
                        self.budget
                            .combine(&result, &self.compile(inner)?, op_function::or)?;
                }
                Ok(result)
            }
        }
    }

    #[cfg(test)]
    fn example(&self, condition: &Bdd) -> String {
        let assignment = condition
            .first_valuation()
            .expect("a projection is satisfiable");
        Predicate::example(&self.atoms, assignment.as_vector())
    }
}

impl CfgBudget {
    fn combine(
        self,
        left: &Bdd,
        right: &Bdd,
        operation: fn(Option<bool>, Option<bool>) -> Option<bool>,
    ) -> Result<Bdd, ProjectionError> {
        // A small result can still require a large intermediate search. Bound
        // both the operation's task count and its resulting node count.
        Bdd::check_binary_op(self.bdd_tasks, left, right, operation)
            .ok_or(ProjectionError::BddBudget)?;
        Bdd::binary_op_with_limit(self.bdd_nodes, left, right, operation)
            .ok_or(ProjectionError::BddBudget)
    }
}

/// Cover every distinct nonempty fragment selection when affordable, otherwise sample configurations.
pub(super) fn project<'a>(
    guards: impl IntoIterator<Item = &'a Predicate>,
    constraints: impl IntoIterator<Item = &'a Predicate>,
    budget: CfgBudget,
) -> ProjectionReport {
    // Keep source-order indices stable for warnings and fragment identity.
    let guards = Vec::from_iter(guards);
    let constraints = Vec::from_iter(constraints);
    if guards.is_empty() {
        return ProjectionReport {
            projections: Vec::new(),
            warning: None,
        };
    }
    // Attempt full symbolic coverage before falling back to concrete samples.
    match project_bdd(&guards, &constraints, budget) {
        Ok(report) => report,
        Err(warning) => ProjectionReport {
            projections: sample_projections(&guards, &constraints, budget.samples),
            warning: Some(warning),
        },
    }
}

/// Partition valid configurations by fragment identity without resolving fragment text.
fn project_bdd(
    guards: &[&Predicate],
    constraints: &[&Predicate],
    budget: CfgBudget,
) -> Result<ProjectionReport, ProjectionWarning> {
    // Intern repeated atoms once and track which source predicate exhausts the budget.
    let mut atoms = Vec::new();
    let mut atom_sites = Vec::new();
    let predicates = constraints
        .iter()
        .enumerate()
        .map(|(index, predicate)| (ProjectionSite::Constraint(index), predicate))
        .chain(
            guards
                .iter()
                .enumerate()
                .map(|(index, predicate)| (ProjectionSite::Guard(index), predicate)),
        );
    for (site, predicate) in predicates {
        predicate
            .collect_atoms(&mut atoms, budget.variable_limit())
            .map_err(|error| ProjectionWarning { error, site })?;
        atom_sites.resize(atoms.len(), site);
    }
    // Share one variable order so repeated predicates stay correlated across all guards.
    let variables = BddVariableSet::new_anonymous(atoms.len() as u16);
    let space = ConditionSpace {
        atoms,
        variables,
        budget,
    };
    // Conjoin owner constraints before exploring which documentation fragments are selected.
    let mut valid = space.variables.mk_true();
    for relation in atom_relations(space.atoms.iter()) {
        let [(left, left_value), (right, right_value)] = relation.clause();
        let literal = |index, value| {
            let variable = space.variables.mk_var(BddVariable::from_index(index));
            if value {
                variable
            } else {
                variable.into_not()
            }
        };
        let warning = |error| ProjectionWarning {
            error,
            site: atom_sites[left.max(right)],
        };
        let clause = budget
            .combine(
                &literal(left, left_value),
                &literal(right, right_value),
                op_function::or,
            )
            .map_err(warning)?;
        valid = budget
            .combine(&valid, &clause, op_function::and)
            .map_err(warning)?;
    }
    for (index, constraint) in constraints.iter().enumerate() {
        let warning = |error| ProjectionWarning {
            error,
            site: ProjectionSite::Constraint(index),
        };
        let constraint = space.compile(constraint).map_err(warning)?;
        valid = budget
            .combine(&valid, &constraint, op_function::and)
            .map_err(warning)?;
        if valid.is_false() {
            return Ok(ProjectionReport {
                projections: Vec::new(),
                warning: None,
            });
        }
    }

    // A zero region budget opts directly into sampled coverage.
    if budget.variants == 0 {
        return Ok(project_regions(
            &space,
            &[],
            guards,
            constraints,
            Some(ProjectionWarning {
                error: ProjectionError::Variants { limit: 0 },
                site: ProjectionSite::Guard(0),
            }),
        ));
    }
    // Split satisfiable regions by fragment guards instead of enumerating assignments.
    let mut regions = vec![valid];
    for (index, guard) in guards.iter().enumerate() {
        let warning = |error| ProjectionWarning {
            error,
            site: ProjectionSite::Guard(index),
        };
        // Preserve known-valid regions if compiling this guard exceeds the budget.
        let guard = match space.compile(guard) {
            Ok(guard) => guard,
            Err(error) => {
                return Ok(project_regions(
                    &space,
                    &regions,
                    guards,
                    constraints,
                    Some(warning(error)),
                ))
            }
        };
        // TODO: consider warning when a documentation guard is unreachable under
        // the owner's constraints, without warning for ordinary always-true guards.
        let mut next = Vec::new();
        for condition in &regions {
            // Separate configurations that include this fragment from those that exclude it.
            let split = || -> Result<_, ProjectionError> {
                Ok((
                    budget.combine(condition, &guard, op_function::and)?,
                    budget.combine(condition, &guard, op_function::and_not)?,
                ))
            };
            let (included, excluded) = match split() {
                Ok(split) => split,
                Err(error) => {
                    return Ok(project_regions(
                        &space,
                        &regions,
                        guards,
                        constraints,
                        Some(warning(error)),
                    ))
                }
            };
            // Impossible branches cannot contribute a fragment selection.
            if !excluded.is_false() {
                next.push(excluded);
            }
            if !included.is_false() {
                next.push(included);
            }
            // Keep the previous partition when further splitting would exceed the cap.
            if next.len() > budget.variants {
                return Ok(project_regions(
                    &space,
                    &regions,
                    guards,
                    constraints,
                    Some(warning(ProjectionError::Variants {
                        limit: budget.variants,
                    })),
                ));
            }
        }
        regions = next;
    }
    Ok(project_regions(&space, &regions, guards, constraints, None))
}

/// Evaluate all fragment guards for each retained region's witness, even after overflow.
fn project_regions(
    space: &ConditionSpace,
    regions: &[Bdd],
    guards: &[&Predicate],
    constraints: &[&Predicate],
    warning: Option<ProjectionWarning>,
) -> ProjectionReport {
    // Exact extraction covers every region; only incomplete coverage uses the sample cap.
    let limit = warning
        .as_ref()
        .map_or(regions.len(), |_| space.budget.samples);
    if limit == 0 {
        return ProjectionReport {
            projections: Vec::new(),
            warning,
        };
    }
    // Retained BDD valuations use this exact atom order, not a reconstructed traversal order.
    let atoms = IndexSet::from_iter(space.atoms.iter().cloned());
    let mut collector = ProjectionCollector::with_atoms(guards, constraints, atoms, limit);
    for region in regions.iter().take(limit) {
        let assignment = region
            .first_valuation()
            .expect("a retained region is satisfiable");
        collector.record(assignment.into_vector());
    }
    // Reuse the same assignment ledger so retained witnesses are not sampled twice.
    if warning.is_some() {
        collector.sample_remaining();
    }
    ProjectionReport {
        projections: collector.projections,
        warning,
    }
}

/// Evaluate bounded concrete configurations, preserving every selected doc fragment.
fn sample_projections(
    guards: &[&Predicate],
    constraints: &[&Predicate],
    limit: usize,
) -> Vec<Projection> {
    let mut collector = ProjectionCollector::new(guards, constraints, limit);
    collector.sample_remaining();
    collector.projections
}

/// Records distinct complete cfg assignments and their source-fragment selections within one budget.
struct ProjectionCollector<'a> {
    guards: &'a [&'a Predicate],
    constraints: &'a [&'a Predicate],
    atoms: IndexSet<Atom>,
    relations: Vec<AtomRelation>,
    assignments: HashSet<Vec<u64>>,
    selections: HashSet<Vec<usize>>,
    projections: Vec<Projection>,
    limit: usize,
}

impl<'a> ProjectionCollector<'a> {
    fn new(guards: &'a [&'a Predicate], constraints: &'a [&'a Predicate], limit: usize) -> Self {
        let mut atoms = IndexSet::new();
        // Zero sampling disables atom interning as well as configuration evaluation.
        if limit > 0 {
            for predicate in constraints.iter().chain(guards) {
                predicate.visit_atoms(&mut |atom| {
                    atoms.insert(atom.clone());
                });
            }
        }
        Self::with_atoms(guards, constraints, atoms, limit)
    }

    fn with_atoms(
        guards: &'a [&'a Predicate],
        constraints: &'a [&'a Predicate],
        atoms: IndexSet<Atom>,
        limit: usize,
    ) -> Self {
        let relations = atom_relations(atoms.iter());
        Self {
            guards,
            constraints,
            atoms,
            relations,
            assignments: HashSet::new(),
            selections: HashSet::new(),
            projections: Vec::new(),
            limit,
        }
    }

    fn is_full(&self) -> bool {
        self.assignments.len() >= self.limit
    }

    fn record(&mut self, assignment: Vec<bool>) {
        // Charge only distinct attempted configurations, including rejected ones.
        if self.is_full() {
            return;
        }
        debug_assert_eq!(assignment.len(), self.atoms.len());
        if !self.assignments.insert(packed_assignment(&assignment)) {
            return;
        }
        if self.relations.iter().any(|relation| {
            !relation
                .clause()
                .iter()
                .any(|&(index, value)| assignment[index] == value)
        }) {
            return;
        }
        let value = |atom: &Atom| assignment[self.atoms.get_index_of(atom).unwrap()];
        if !self
            .constraints
            .iter()
            .all(|predicate| predicate.evaluate(&value))
        {
            return;
        }
        // Always evaluate every guard, including fragments after symbolic exploration stopped.
        let selected = Vec::from_iter(
            self.guards
                .iter()
                .enumerate()
                .filter(|(_, guard)| guard.evaluate(&value))
                .map(|(index, _)| index),
        );
        if !selected.is_empty() && self.selections.insert(selected.clone()) {
            self.projections.push(Projection {
                selected,
                example: Predicate::example(&self.atoms, &assignment),
            });
        }
    }

    fn sample_remaining(&mut self) {
        if self.is_full() {
            return;
        }
        let width = self.atoms.len();
        // Try both endpoints before exploring individual variables.
        for background in [false, true] {
            self.record(vec![background; width]);
            if self.is_full() {
                return;
            }
        }
        // Single flips cover sparse and nearly fully enabled configurations.
        for index in 0..width.min(128) {
            for background in [false, true] {
                let mut assignment = vec![background; width];
                assignment[index] = !background;
                self.record(assignment);
                if self.is_full() {
                    return;
                }
            }
        }
        // Bound candidate generation even when a small domain has already been exhausted.
        let patterns = if width < usize::BITS as usize {
            (1usize << width).min(self.limit.saturating_mul(2))
        } else {
            self.limit.saturating_mul(2)
        };
        for pattern in 0..patterns {
            for background in [false, true] {
                let mut assignment = vec![background; width];
                for (index, value) in assignment.iter_mut().take(usize::BITS as usize).enumerate() {
                    *value = background ^ (pattern & (1 << index) != 0);
                }
                self.record(assignment);
                if self.is_full() {
                    return;
                }
            }
        }
    }
}

/// A binary clause shared by symbolic reasoning and concrete fallback validation.
#[derive(Clone, Copy)]
enum AtomRelation {
    Implies(usize, usize),
    Excludes(usize, usize),
}

impl AtomRelation {
    fn clause(self) -> [(usize, bool); 2] {
        match self {
            Self::Implies(left, right) => [(left, false), (right, true)],
            Self::Excludes(left, right) => [(left, false), (right, false)],
        }
    }
}

/// Encode only known relationships among observed atoms, without choosing a host target.
fn atom_relations<'a>(atoms: impl IntoIterator<Item = &'a Atom>) -> Vec<AtomRelation> {
    let atoms = Vec::from_iter(atoms);
    let versions = ordered_version_indices(atoms.iter().copied());
    // Adjacent implications suffice: every newer threshold entails every older one.
    let mut relations = Vec::from_iter(
        versions
            .windows(2)
            .map(|pair| AtomRelation::Implies(pair[1], pair[0])),
    );
    let mut unix = Vec::new();
    let mut windows = Vec::new();
    for (index, atom) in atoms.iter().enumerate() {
        let family = match atom {
            Atom::Flag(name) => name.as_str(),
            Atom::Value(key, value) if key == "target_family" => value.as_str(),
            _ => continue,
        };
        match family {
            "unix" => unix.push(index),
            "windows" => windows.push(index),
            _ => {}
        }
    }
    for family in [&unix, &windows] {
        for pair in family.windows(2) {
            relations.push(AtomRelation::Implies(pair[0], pair[1]));
            relations.push(AtomRelation::Implies(pair[1], pair[0]));
        }
    }
    // These families exclude one another, but neither is required. Other families
    // are multi-valued (e.g. unix + wasm) and must not be made mutually exclusive.
    for &left in &unix {
        for &right in &windows {
            relations.push(AtomRelation::Excludes(left, right));
        }
    }
    relations
}

/// Return atom indices in increasing release order for BDD constraints and concrete samples.
fn ordered_version_indices<'a>(atoms: impl IntoIterator<Item = &'a Atom>) -> Vec<usize> {
    let mut versions = Vec::from_iter(atoms.into_iter().enumerate().filter_map(|(index, atom)| {
        if let Atom::Version(version) = atom {
            Some((*version, index))
        } else {
            None
        }
    }));
    versions.sort_unstable();
    Vec::from_iter(versions.into_iter().map(|(_, index)| index))
}

/// Encode a fixed-width assignment exactly while avoiding a byte per Boolean in the ledger.
fn packed_assignment(assignment: &[bool]) -> Vec<u64> {
    let width = u64::BITS as usize;
    let mut words = vec![0; assignment.len().div_ceil(width)];
    for (index, enabled) in assignment.iter().enumerate() {
        if *enabled {
            words[index / width] |= 1 << (index % width);
        }
    }
    words
}
