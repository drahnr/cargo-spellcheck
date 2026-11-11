//! Symbolic configuration predicates used to project conditional documentation.
//!
//! We deliberately do not evaluate the host's configuration: documentation for
//! other targets and disabled features needs spellchecking as well.

use biodivine_lib_bdd::{op_function, Bdd, BddVariable, BddVariableSet};
use syn::ext::IdentExt;
use syn::punctuated::Punctuated;
use syn::{Expr, Lit, LitBool, Meta, Token};

/// A Boolean cfg flag or key-value pair treated as one symbolic variable.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Atom {
    Flag(String),
    Value(String, String),
}

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
        if input.peek(LitBool) {
            return Ok(Self::Constant(input.parse::<LitBool>()?.value));
        }
        Self::from_meta(input.parse()?)
    }
}

impl Atom {
    fn as_predicate(&self) -> String {
        let name = match self {
            Self::Flag(name) | Self::Value(name, _) => name,
        };
        let name = if syn::parse_str::<syn::Ident>(name).is_ok() {
            name.clone()
        } else {
            format!("r#{name}")
        };
        match self {
            Self::Flag(_) => name,
            Self::Value(_, value) => format!("{name} = {value:?}"),
        }
    }
}

impl Predicate {
    pub(super) fn always() -> Self {
        Self::Constant(true)
    }

    fn from_meta(meta: Meta) -> syn::Result<Self> {
        let name = meta
            .path()
            .get_ident()
            .map(|ident| ident.unraw().to_string());
        match (name.as_deref(), meta) {
            (Some(name), Meta::Path(_)) => Ok(Self::Atom(Atom::Flag(name.to_owned()))),
            (Some(name), Meta::NameValue(value)) => match value.value {
                Expr::Lit(syn::ExprLit {
                    lit: Lit::Str(value),
                    ..
                }) => Ok(Self::Atom(Atom::Value(name.to_owned(), value.value()))),
                other => Err(syn::Error::new_spanned(
                    other,
                    "expected a cfg string value",
                )),
            },
            (Some("all" | "any" | "not"), Meta::List(list)) => {
                let predicates = list
                    .parse_args_with(Punctuated::<Predicate, Token![,]>::parse_terminated)?
                    .into_iter()
                    .collect::<Vec<_>>();
                match name.as_deref() {
                    Some("all") => Ok(Self::All(predicates)),
                    Some("any") => Ok(Self::Any(predicates)),
                    _ if predicates.len() == 1 => {
                        Ok(Self::Not(Box::new(predicates.into_iter().next().unwrap())))
                    }
                    _ => Err(syn::Error::new_spanned(
                        list.path,
                        "not() requires exactly one predicate",
                    )),
                }
            }
            (_, other) => Err(syn::Error::new_spanned(other, "unsupported cfg predicate")),
        }
    }

    /// Describe one complete assignment over the atoms relevant to a run.
    fn example(atoms: &[Atom], assignment: &[bool]) -> String {
        let mut terms: Vec<_> = atoms
            .iter()
            .enumerate()
            .map(|(index, atom)| {
                let predicate = atom.as_predicate();
                if assignment[index] {
                    predicate
                } else {
                    format!("not({predicate})")
                }
            })
            .collect();
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

/// Per-run limits for symbolic cfg reasoning and fallback configuration sampling.
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

/// A distinct selection of source literals and one configuration producing it.
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

/// An incomplete-coverage warning located at a documentation guard or item constraint.
#[derive(Debug)]
pub(super) struct ProjectionWarning {
    pub(super) error: ProjectionError,
    pub(super) site: ProjectionSite,
}

/// Complete documentation projections, with a warning when coverage was sampled.
pub(super) struct ProjectionReport {
    pub(super) projections: Vec<Projection>,
    pub(super) warning: Option<ProjectionWarning>,
}

/// Maps one documentation run's cfg atoms to a shared BDD variable order and enforces its budget.
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
            Predicate::All(predicates) | Predicate::Any(predicates) => {
                let conjunction = matches!(predicate, Predicate::All(_));
                let mut result = if conjunction {
                    self.variables.mk_true()
                } else {
                    self.variables.mk_false()
                };
                for inner in predicates {
                    // Constants and tautologies can determine the result without
                    // compiling later, potentially expensive subexpressions.
                    if (conjunction && result.is_false()) || (!conjunction && result.is_true()) {
                        break;
                    }
                    let inner = self.compile(inner)?;
                    result = self.budget.combine(
                        &result,
                        &inner,
                        if conjunction {
                            op_function::and
                        } else {
                            op_function::or
                        },
                    )?;
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

/// Cover every distinct literal selection when affordable, otherwise check a sample.
pub(super) fn project<'a>(
    guards: impl IntoIterator<Item = &'a Predicate>,
    constraints: impl IntoIterator<Item = &'a Predicate>,
    budget: CfgBudget,
) -> ProjectionReport {
    // Keep source-order indices stable for warnings and literal identity.
    let guards: Vec<_> = guards.into_iter().collect();
    let constraints: Vec<_> = constraints.into_iter().collect();
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

/// Partition valid configurations by literal identity rather than rendered text.
fn project_bdd(
    guards: &[&Predicate],
    constraints: &[&Predicate],
    budget: CfgBudget,
) -> Result<ProjectionReport, ProjectionWarning> {
    // Intern repeated atoms once and track which source predicate exhausts the budget.
    let mut atoms = Vec::new();
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
    }
    // Share one variable order so repeated predicates stay correlated across all guards.
    let variables = BddVariableSet::new_anonymous(atoms.len() as u16);
    let space = ConditionSpace {
        atoms,
        variables,
        budget,
    };
    // Conjoin item constraints before exploring which documentation is visible.
    let mut valid = space.variables.mk_true();
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
    // Split satisfiable regions by literal guards instead of enumerating assignments.
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
        // the item's constraints, without warning for ordinary always-true guards.
        let mut next = Vec::new();
        for condition in &regions {
            // Separate configurations that include this literal from those that exclude it.
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
            // Impossible branches cannot contribute a documentation variant.
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

/// Complete every retained region's witness using all guards, even after overflow.
fn project_regions(
    space: &ConditionSpace,
    regions: &[Bdd],
    guards: &[&Predicate],
    constraints: &[&Predicate],
    warning: Option<ProjectionWarning>,
) -> ProjectionReport {
    // Reuse the symbolic variable order when evaluating concrete witnesses.
    let indices: indexmap::IndexSet<_> = space.atoms.iter().cloned().collect();
    let mut seen = std::collections::HashSet::new();
    let mut projections = Vec::new();
    // Cap sampled coverage without truncating successful exact extraction.
    let retained = if warning.is_some() {
        regions.len().min(space.budget.samples)
    } else {
        regions.len()
    };
    // Pick one complete satisfying assignment from each retained region.
    for region in regions.iter().take(retained) {
        let assignment = region
            .first_valuation()
            .expect("a retained region is satisfiable");
        let value = |atom: &Atom| assignment.as_vector()[indices.get_index_of(atom).unwrap()];
        // Evaluate all guards so overflow cannot omit later fragments such as code fences.
        let selected: Vec<_> = guards
            .iter()
            .enumerate()
            .filter(|(_, guard)| guard.evaluate(&value))
            .map(|(index, _)| index)
            .collect();
        if !selected.is_empty() && seen.insert(selected.clone()) {
            projections.push(Projection {
                selected,
                example: Predicate::example(&space.atoms, assignment.as_vector()),
            });
        }
    }
    // Spend only the remaining sample budget on configurations not guaranteed valid.
    if warning.is_some() {
        for projection in sample_projections(
            guards,
            constraints,
            space.budget.samples.saturating_sub(retained),
        ) {
            if seen.insert(projection.selected.clone()) {
                projections.push(projection);
            }
        }
    }
    ProjectionReport {
        projections,
        warning,
    }
}

/// Evaluate bounded concrete configurations, preserving every selected doc fragment.
fn sample_projections(
    guards: &[&Predicate],
    constraints: &[&Predicate],
    limit: usize,
) -> Vec<Projection> {
    if limit == 0 {
        return Vec::new();
    }
    // Include atoms beyond the symbolic limit so sampled assignments have no unknowns.
    let mut atoms = indexmap::IndexSet::new();
    for predicate in constraints.iter().chain(guards) {
        predicate.visit_atoms(&mut |atom| {
            atoms.insert(atom.clone());
        });
    }
    let names: Vec<_> = atoms.iter().cloned().collect();
    let mut assignment = vec![false; atoms.len()];
    let mut seen = std::collections::HashSet::new();
    let mut projections = Vec::new();
    // Try endpoints, single flips, then low-variable combinations against both backgrounds.
    let individual_samples = atoms.len().min(128) * 2;
    for sample in 0..limit {
        if sample < 2 {
            // Start with all-disabled and all-enabled configurations.
            assignment.fill(sample == 1);
        } else if sample < individual_samples + 2 {
            // Probe one variable at a time against both backgrounds.
            let flip = sample - 2;
            assignment.fill(flip % 2 == 1);
            assignment[flip / 2] ^= true;
        } else {
            // Mix low-order variables while fixing every remaining variable.
            let pattern = sample - individual_samples - 2;
            let background = pattern % 2 == 1;
            assignment.fill(background);
            for (index, value) in assignment.iter_mut().take(usize::BITS as usize).enumerate() {
                *value = background ^ ((pattern / 2) & (1 << index) != 0);
            }
        }
        let value = |atom: &Atom| assignment[atoms.get_index_of(atom).unwrap()];
        // Reject configurations violating item constraints before selecting documentation.
        if !constraints
            .iter()
            .all(|predicate| predicate.evaluate(&value))
        {
            continue;
        }
        let selected: Vec<_> = guards
            .iter()
            .enumerate()
            .filter(|(_, guard)| guard.evaluate(&value))
            .map(|(index, _)| index)
            .collect();
        // Deduplicate source selections, not text shared by different source locations.
        if !selected.is_empty() && seen.insert(selected.clone()) {
            projections.push(Projection {
                selected,
                example: Predicate::example(&names, &assignment),
            });
        }
    }
    projections
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> syn::Result<Predicate> {
        syn::parse_str(source)
    }

    #[test]
    fn example_roundtrips_as_exactly_one_assignment() {
        let source = r###"all(flag, feature = "a\\\"b", r#false)"###;
        let predicate = parse(source).unwrap();
        let mut atoms = Vec::new();
        let atom_limit = CfgBudget::default().variables;
        predicate.collect_atoms(&mut atoms, atom_limit).unwrap();
        for assignment in 0..(1 << atoms.len()) {
            let values: Vec<_> = (0..atoms.len())
                .map(|index| assignment & (1 << index) != 0)
                .collect();
            let example = Predicate::example(&atoms, &values);
            let example = parse(&example).unwrap();
            for other in 0..(1 << atoms.len()) {
                let expected_match = assignment == other;
                assert_eq!(example.matches(&atoms, other), expected_match);
            }
        }
        let empty_atoms = [];
        let empty_values = [];
        let expected_example = "all()";
        assert_eq!(
            Predicate::example(&empty_atoms, &empty_values),
            expected_example
        );
    }

    #[test]
    fn boolean_literals_do_not_create_atoms() {
        // Cases pair a cfg predicate with its expected constant truth value.
        let cases = [
            ("true", true),
            ("false", false),
            ("not(false)", true),
            ("all(true, not(false))", true),
            ("any(false, not(true))", false),
        ];
        let atom_limit = CfgBudget::default().variables;
        for (source, expected) in cases {
            let predicate = parse(source).unwrap();
            let mut atoms = Vec::new();
            predicate.collect_atoms(&mut atoms, atom_limit).unwrap();
            assert!(atoms.is_empty(), "{source}");
            assert_eq!(predicate.matches(&atoms, 0), expected, "{source}");
        }
    }

    #[test]
    fn nested_predicates_keep_repeated_atoms_correlated() {
        let source = r#"all(any(feature = "a", unix,), not(feature = "a"),)"#;
        let predicate = parse(source).unwrap();
        let mut atoms = Vec::new();
        let atom_limit = CfgBudget::default().variables;
        predicate.collect_atoms(&mut atoms, atom_limit).unwrap();
        let expected_atoms = vec![
            Atom::Value("feature".into(), "a".into()),
            Atom::Flag("unix".into()),
        ];
        assert_eq!(atoms, expected_atoms);
        // Cases pair an assignment bitmask with the expected predicate result.
        let cases = [(0, false), (1, false), (2, true), (3, false)];
        for (assignment, expected_match) in cases {
            assert_eq!(predicate.matches(&atoms, assignment), expected_match);
        }
    }

    #[test]
    fn raw_predicate_names_share_atoms_with_plain_names() {
        let source = r#"all(r#unix, unix, r#feature = "a", feature = "a")"#;
        let predicate = parse(source).unwrap();
        let mut atoms = Vec::new();
        let atom_limit = CfgBudget::default().variables;
        predicate.collect_atoms(&mut atoms, atom_limit).unwrap();
        let expected_atoms = vec![
            Atom::Flag("unix".into()),
            Atom::Value("feature".into(), "a".into()),
        ];
        assert_eq!(atoms, expected_atoms);
        let assignment = 3;
        assert!(predicate.matches(&atoms, assignment));
    }

    #[test]
    fn bdd_translation_matches_an_independent_truth_table() {
        // Each input predicate is checked against every assignment of its atoms.
        let cases = [
            "true",
            "false",
            "all()",
            "any()",
            "any(online, not(online))",
            "all(cached, not(cached))",
            "not(any(all(read, write), not(all(read, seek))))",
            "all(any(alpha, beta), any(not(alpha), gamma), not(all(beta, gamma)))",
            r####"any(all(r#unix, feature = r#"one"#), all(not(unix), feature = "a\\\"b"))"####,
            r#"all(été, any(feature = "café", not(feature = "café")))"#,
        ];
        for source in cases {
            let predicate = parse(source).unwrap();
            let space = ConditionSpace::new(std::iter::once(&predicate)).unwrap();
            let bdd = space.compile(&predicate).unwrap();
            for assignment in 0..(1 << space.atoms.len()) {
                let values = (0..space.atoms.len())
                    .map(|index| assignment & (1 << index) != 0)
                    .collect();
                let valuation = biodivine_lib_bdd::BddValuation::new(values);
                let expected_match = predicate.matches(&space.atoms, assignment);
                assert_eq!(
                    bdd.eval_in(&valuation),
                    expected_match,
                    "{source}, assignment {assignment}",
                );
            }
        }
    }

    #[test]
    fn bdd_reduces_complementary_branches_and_absorbs_contradictions() {
        // Cases pair an original cfg predicate with its expected reduced predicate.
        let cases = [
            (r#"any(all(not(a), b), all(not(a), not(b)))"#, r#"not(a)"#),
            (r#"any(all(not(a), b), all(not(a), not(b)), a)"#, r#"true"#),
            (r#"any(a, not(a))"#, r#"true"#),
            (r#"all(a, not(a))"#, r#"false"#),
            (r#"any(all(a, not(a)), b)"#, r#"b"#),
        ];
        for (source, expected_source) in cases {
            let original = parse(source).unwrap();
            let expected = parse(expected_source).unwrap();
            let space = ConditionSpace::new([&original, &expected].into_iter()).unwrap();
            let original = space.compile(&original).unwrap();
            let expected = space.compile(&expected).unwrap();
            assert_eq!(
                original, expected,
                "{source} must reduce to {expected_source}"
            );
        }
    }

    #[test]
    fn partitions_and_witnesses_match_exhaustive_selection_oracle() {
        /// A documentation run whose projections are checked against exhaustive evaluation.
        struct Case {
            /// Scenario name included in assertion failures.
            name: &'static str,
            /// One predicate per source literal; its slice position is the literal's identity.
            doc_guards: &'static [&'static str],
            /// Predicates that must all hold for the item to exist; an empty slice allows any cfg.
            item_constraints: &'static [&'static str],
        }
        let cases = [
            Case {
                name: "nested optional transport documentation",
                doc_guards: &["true", "any(tls, unix)", "all(tls, unix)"],
                item_constraints: &[],
            },
            Case {
                name: "equal guards retain distinct source literals",
                doc_guards: &["cached", "cached", "not(cached)", "false"],
                item_constraints: &[],
            },
            Case {
                name: "remote access requires authentication or local auditing",
                doc_guards: &[
                    "true",
                    "all(remote, authenticated)",
                    "any(authenticated, audited)",
                    "not(audited)",
                ],
                item_constraints: &["any(not(remote), authenticated)", "any(remote, audited)"],
            },
            Case {
                name: "contradictory item constraints remove all documentation",
                doc_guards: &["enabled", "not(enabled)"],
                item_constraints: &["enabled", "not(enabled)"],
            },
            Case {
                name: "an impossible literal guard selects no documentation",
                doc_guards: &["all(online, not(online))"],
                item_constraints: &[],
            },
        ];
        for Case {
            name,
            doc_guards,
            item_constraints,
        } in cases
        {
            let guards = Vec::from_iter(doc_guards.iter().map(|source| parse(source).unwrap()));
            let constraints =
                Vec::from_iter(item_constraints.iter().map(|source| parse(source).unwrap()));
            let space = ConditionSpace::new(constraints.iter().chain(guards.iter())).unwrap();
            let selection = |assignment| {
                let selected = guards
                    .iter()
                    .enumerate()
                    .filter(|(_, guard)| guard.matches(&space.atoms, assignment))
                    .map(|(index, _)| index);
                Vec::from_iter(selected)
            };
            let valid = |assignment| {
                constraints
                    .iter()
                    .all(|constraint| constraint.matches(&space.atoms, assignment))
            };
            let mut expected = std::collections::BTreeSet::new();
            for assignment in 0..(1 << space.atoms.len()) {
                let selected = selection(assignment);
                if valid(assignment) && !selected.is_empty() {
                    expected.insert(selected);
                }
            }
            let budget = CfgBudget::default();
            let report = project(guards.iter(), &constraints, budget);
            assert!(report.warning.is_none());
            let projections = report.projections;
            let actual = std::collections::BTreeSet::from_iter(
                projections
                    .iter()
                    .map(|projection| projection.selected.clone()),
            );
            assert_eq!(actual, expected, "{name}");
            let expected_projection_count = actual.len();
            assert_eq!(
                projections.len(),
                expected_projection_count,
                "{name}: duplicate literal selection"
            );
            for projection in projections {
                let example = parse(&projection.example).unwrap();
                let assignments = Vec::from_iter(
                    (0..(1 << space.atoms.len()))
                        .filter(|assignment| example.matches(&space.atoms, *assignment)),
                );
                let expected_assignment_count = 1;
                assert_eq!(
                    assignments.len(),
                    expected_assignment_count,
                    "{name}: witness must be a complete assignment"
                );
                assert!(
                    valid(assignments[0]),
                    "{name}: witness violates item constraints"
                );
                let expected_selection = selection(assignments[0]);
                assert_eq!(projection.selected, expected_selection, "{name}");
            }
        }
    }

    #[test]
    fn witnesses_support_more_atoms_than_a_machine_word() {
        let predicate = Predicate::All(
            (0..128)
                .map(|index| Predicate::Atom(Atom::Flag(format!("sensor_{index}"))))
                .collect(),
        );
        let space = ConditionSpace::new(std::iter::once(&predicate)).unwrap();
        let constraints = [];
        let budget = CfgBudget::default();
        let report = project(std::iter::once(&predicate), &constraints, budget);
        assert!(report.warning.is_none());
        let projections = report.projections;
        let expected_count = 1;
        assert_eq!(projections.len(), expected_count);
        let example = parse(&projections[0].example).unwrap();
        assert!(space
            .compile(&example)
            .unwrap()
            .iff(&space.compile(&predicate).unwrap())
            .is_true());
    }

    #[test]
    fn bdds_and_condition_spaces_can_be_shared_across_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Bdd>();
        assert_send_sync::<ConditionSpace>();
        assert_send_sync::<Projection>();

        let predicate = parse(r#"all(any(feature = "tls", unix), not(embedded))"#).unwrap();
        let space = ConditionSpace::new(std::iter::once(&predicate)).unwrap();
        let bdd = space.compile(&predicate).unwrap();
        let example = space.example(&bdd);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    let independently_compiled = space.compile(&predicate).unwrap();
                    assert!(bdd.iff(&independently_compiled).is_true());
                    assert_eq!(space.example(&bdd), example);
                });
            }
        });
    }

    #[test]
    fn default_budget_preserves_existing_limits() {
        let budget = CfgBudget::default();
        let expected = CfgBudget {
            variables: 1024,
            bdd_nodes: 4096,
            bdd_tasks: 16384,
            variants: 1024,
            samples: 1024,
        };
        assert_eq!(budget, expected);
    }

    #[test]
    fn variable_limit_clamps_to_the_library_representation() {
        let budget = CfgBudget {
            variables: usize::MAX,
            ..CfgBudget::default()
        };
        let expected_limit = usize::from(u16::MAX) - 2;
        assert_eq!(budget.variable_limit(), expected_limit);
    }

    #[test]
    fn variable_budget_accepts_the_boundary_and_rejects_overflow() {
        let budget = CfgBudget {
            variables: 2,
            ..CfgBudget::default()
        };
        let predicates: Vec<_> = (0..=budget.variables)
            .map(|index| Predicate::Atom(Atom::Flag(format!("input_{index}"))))
            .collect();
        let boundary = &predicates[..budget.variables];
        assert!(ConditionSpace::with_budget(boundary.iter(), budget).is_ok());
        assert!(matches!(
            ConditionSpace::with_budget(predicates.iter(), budget),
            Err(ProjectionError::Variables { .. })
        ));
    }

    #[test]
    fn one_large_predicate_stops_collecting_at_the_variable_budget() {
        let atom_limit = 3;
        let predicate = Predicate::Any(
            (0..(atom_limit * 2))
                .map(|index| Predicate::Atom(Atom::Flag(format!("sensor_{index}"))))
                .collect(),
        );
        let mut atoms = Vec::new();
        assert!(matches!(
            predicate.collect_atoms(&mut atoms, atom_limit),
            Err(ProjectionError::Variables { .. })
        ));
        let expected_atom_count = atom_limit;
        assert_eq!(atoms.len(), expected_atom_count);
    }

    fn request_response_budget_predicates() -> (Predicate, Predicate) {
        // Reserve requests before responses to force exponential equality BDD growth.
        let ordering_source = r#"all(
            any(request_0, not(request_0)), any(request_1, not(request_1)),
            any(request_2, not(request_2)), any(response_0, not(response_0)),
            any(response_1, not(response_1)), any(response_2, not(response_2))
        )"#;
        let equality_source = r#"all(
            any(all(request_0, response_0), all(not(request_0), not(response_0))),
            any(all(request_1, response_1), all(not(request_1), not(response_1))),
            any(all(request_2, response_2), all(not(request_2), not(response_2)))
        )"#;
        (
            parse(ordering_source).unwrap(),
            parse(equality_source).unwrap(),
        )
    }

    fn assert_valid_projections(
        guards: &[Predicate],
        constraints: &[Predicate],
        projections: &[Projection],
        expected_max_samples: usize,
    ) {
        let mut expected_atoms = Vec::new();
        for predicate in constraints.iter().chain(guards) {
            predicate.visit_atoms(&mut |atom| {
                if !expected_atoms.contains(atom) {
                    expected_atoms.push(atom.clone());
                }
            });
        }
        let mut selections = std::collections::HashSet::new();
        for projection in projections {
            let source = &projection.example;
            let witness = parse(source).unwrap();
            let terms = match witness {
                Predicate::All(terms) => terms,
                term => vec![term],
            };
            let assignment: Vec<_> = terms
                .into_iter()
                .map(|term| match term {
                    Predicate::Atom(atom) => (atom, true),
                    Predicate::Not(inner) => match *inner {
                        Predicate::Atom(atom) => (atom, false),
                        _ => panic!("witness contains a non-literal: {source}"),
                    },
                    _ => panic!("witness contains a non-literal: {source}"),
                })
                .collect();
            let actual_atoms: Vec<_> = assignment.iter().map(|(atom, _)| atom.clone()).collect();
            assert_eq!(actual_atoms, expected_atoms, "incomplete witness: {source}");
            let value = |atom: &Atom| {
                assignment
                    .iter()
                    .find(|(candidate, _)| candidate == atom)
                    .unwrap()
                    .1
            };
            assert!(constraints
                .iter()
                .all(|constraint| constraint.evaluate(&value)));
            let expected_selected: Vec<_> = guards
                .iter()
                .enumerate()
                .filter(|(_, guard)| guard.evaluate(&value))
                .map(|(index, _)| index)
                .collect();
            assert_eq!(projection.selected, expected_selected, "{source}");
            assert!(!projection.selected.is_empty());
            assert!(
                selections.insert(projection.selected.clone()),
                "duplicate selection"
            );
        }
        assert!(projections.len() <= expected_max_samples);
    }

    #[test]
    fn bdd_growth_overflow_retains_complete_sampled_projections() {
        let budget = CfgBudget {
            bdd_nodes: 8,
            samples: 32,
            ..CfgBudget::default()
        };
        let expected_max_samples = budget.samples;
        let (ordering, equality) = request_response_budget_predicates();
        let trailing_source = r#"not(request_0)"#;
        let trailing_guard = parse(trailing_source).unwrap();
        let guards = [ordering, equality, trailing_guard];
        let constraints = [];
        let report = project(guards.iter(), &constraints, budget);
        let warning = report.warning.as_ref().expect("sampled coverage warning");
        let expected_site = ProjectionSite::Guard(1);
        assert!(matches!(warning.error, ProjectionError::BddBudget));
        assert_eq!(warning.site, expected_site);
        let expected_selections =
            std::collections::BTreeSet::from([vec![0], vec![0, 1], vec![0, 1, 2], vec![0, 2]]);
        let actual_selections: std::collections::BTreeSet<_> = report
            .projections
            .iter()
            .map(|projection| projection.selected.clone())
            .collect();
        assert_eq!(actual_selections, expected_selections);
        assert_valid_projections(
            &guards,
            &constraints,
            &report.projections,
            expected_max_samples,
        );
    }

    #[test]
    fn variable_overflow_samples_complete_witnesses_beyond_the_budget() {
        let budget = CfgBudget {
            variables: 2,
            samples: 16,
            ..CfgBudget::default()
        };
        let expected_max_samples = budget.samples;
        let atom_count = budget.variables + 1;
        let sensor_guard = Predicate::Any(
            (0..atom_count)
                .map(|index| Predicate::Atom(Atom::Flag(format!("sensor_{index}"))))
                .collect(),
        );
        let trailing_source = r#"not(sensor_2)"#;
        let trailing_guard = parse(trailing_source).unwrap();
        let constraint_source = r#"not(sensor_0)"#;
        let constraint = parse(constraint_source).unwrap();
        let guards = [Predicate::always(), sensor_guard, trailing_guard];
        let constraints = [constraint];
        let report = project(guards.iter(), &constraints, budget);
        let warning = report.warning.as_ref().expect("sampled coverage warning");
        let expected_site = ProjectionSite::Guard(1);
        assert!(matches!(warning.error, ProjectionError::Variables { .. }));
        assert_eq!(warning.site, expected_site);
        let expected_selections =
            std::collections::BTreeSet::from([vec![0, 1], vec![0, 1, 2], vec![0, 2]]);
        let actual_selections: std::collections::BTreeSet<_> = report
            .projections
            .iter()
            .map(|projection| projection.selected.clone())
            .collect();
        assert_eq!(expected_selections, actual_selections);
        assert_valid_projections(
            &guards,
            &constraints,
            &report.projections,
            expected_max_samples,
        );
    }

    #[test]
    fn constraint_overflow_reports_its_site_and_filters_invalid_samples() {
        let budget = CfgBudget {
            bdd_nodes: 8,
            samples: 32,
            ..CfgBudget::default()
        };
        let expected_max_samples = budget.samples;
        let (ordering, equality) = request_response_budget_predicates();
        let guard_sources = [r#"true"#, r#"request_0"#, r#"not(request_0)"#];
        let guards: Vec<_> = guard_sources
            .iter()
            .map(|source| parse(source).unwrap())
            .collect();
        let constraint_source = r#"not(request_0)"#;
        let trailing_constraint = parse(constraint_source).unwrap();
        let constraints = [ordering, equality, trailing_constraint];
        let report = project(guards.iter(), &constraints, budget);
        let warning = report.warning.as_ref().expect("sampled coverage warning");
        let expected_site = ProjectionSite::Constraint(1);
        assert!(matches!(warning.error, ProjectionError::BddBudget));
        assert_eq!(warning.site, expected_site);
        let expected_selections = vec![vec![0, 2]];
        let actual_selections: Vec<_> = report
            .projections
            .iter()
            .map(|p| p.selected.clone())
            .collect();
        assert_eq!(actual_selections, expected_selections);
        assert_valid_projections(
            &guards,
            &constraints,
            &report.projections,
            expected_max_samples,
        );
    }

    #[test]
    fn variant_overflow_samples_all_guards_instead_of_a_selected_prefix() {
        let budget = CfgBudget {
            variants: 4,
            samples: 16,
            ..CfgBudget::default()
        };
        let expected_max_samples = budget.samples;
        let mut guards: Vec<_> = (0..3)
            .map(|index| Predicate::Atom(Atom::Flag(format!("capability_{index}"))))
            .collect();
        guards.push(Predicate::always());
        let constraints = [];
        let expected_site = ProjectionSite::Guard(2);
        let report = project(guards.iter(), &constraints, budget);
        let warning = report.warning.as_ref().expect("sampled coverage warning");
        assert!(matches!(warning.error, ProjectionError::Variants { .. }));
        assert_eq!(warning.site, expected_site);
        assert!(!report.projections.is_empty());
        assert_valid_projections(
            &guards,
            &constraints,
            &report.projections,
            expected_max_samples,
        );
    }

    #[test]
    fn variant_overflow_preserves_known_valid_regions_missed_by_blind_sampling() {
        let budget = CfgBudget {
            variants: 2,
            samples: 2,
            ..CfgBudget::default()
        };
        let expected_max_samples = budget.samples;
        let constraint_source = r#"all(cache_ready, not(maintenance), remote_ready)"#;
        let constraint = parse(constraint_source).unwrap();
        let mut guards = vec![Predicate::always()];
        guards.extend((0..3).map(|index| Predicate::Atom(Atom::Flag(format!("section_{index}")))));
        // Intern the independent section variables before the constrained
        // variables in the fallback sampler's assignment order.
        let ordering = Predicate::All(
            guards
                .iter()
                .skip(1)
                .map(|guard| {
                    Predicate::Any(vec![guard.clone(), Predicate::Not(Box::new(guard.clone()))])
                })
                .collect(),
        );
        let constraints = [ordering, constraint];
        let guard_refs: Vec<_> = guards.iter().collect();
        let constraint_refs: Vec<_> = constraints.iter().collect();
        let blind_samples = sample_projections(&guard_refs, &constraint_refs, expected_max_samples);
        assert!(blind_samples.is_empty());
        let report = project(guards.iter(), &constraints, budget);
        assert!(matches!(
            report.warning.as_ref().map(|warning| &warning.error),
            Some(ProjectionError::Variants { .. })
        ));
        assert!(!report.projections.is_empty());
        assert_valid_projections(
            &guards,
            &constraints,
            &report.projections,
            expected_max_samples,
        );
    }

    #[test]
    fn fallback_without_a_valid_sample_keeps_warning_but_emits_no_docs() {
        let budget = CfgBudget {
            variables: 2,
            samples: 2,
            ..CfgBudget::default()
        };
        let sensor_guard = Predicate::Any(
            (0..=budget.variables)
                .map(|index| Predicate::Atom(Atom::Flag(format!("sensor_{index}"))))
                .collect(),
        );
        let guards = [Predicate::always(), sensor_guard];
        // The two endpoint samples miss this satisfiable mixed assignment.
        let constraint_source = r#"all(sensor_0, not(sensor_1))"#;
        let unsampled_constraint = parse(constraint_source).unwrap();
        let excluded_atom = Atom::Flag("sensor_1".into());
        let satisfying_value = |atom: &Atom| atom != &excluded_atom;
        assert!(unsampled_constraint.evaluate(&satisfying_value));
        // These item constraints produce no valid configuration in the bounded sample.
        let cases = [unsampled_constraint, Predicate::Constant(false)];
        for constraint in cases {
            let constraints = [constraint];
            let report = project(guards.iter(), &constraints, budget);
            let warning = report
                .warning
                .as_ref()
                .expect("missing sampled coverage warning");
            let expected_site = ProjectionSite::Guard(1);
            assert!(matches!(warning.error, ProjectionError::Variables { .. }));
            assert_eq!(warning.site, expected_site);
            let expected_selections: Vec<Vec<usize>> = Vec::new();
            let actual_selections: Vec<_> = report
                .projections
                .iter()
                .map(|p| p.selected.clone())
                .collect();
            assert_eq!(actual_selections, expected_selections);
        }
    }

    #[test]
    fn fallback_sampling_respects_zero_and_small_sample_budgets() {
        let source = r#"feature = "telemetry""#;
        let guards = [Predicate::always(), parse(source).unwrap()];
        let constraints = [];
        // Cases pair a sample limit with the expected selected source-literal indices.
        let cases = [
            (0, vec![]),
            (1, vec![vec![0]]),
            (2, vec![vec![0], vec![0, 1]]),
        ];
        for (sample_limit, expected_selections) in cases {
            let budget = CfgBudget {
                variables: 0,
                samples: sample_limit,
                ..CfgBudget::default()
            };
            let report = project(guards.iter(), &constraints, budget);
            let warning = report.warning.as_ref().expect("sampled coverage warning");
            let expected_site = ProjectionSite::Guard(1);
            assert!(matches!(warning.error, ProjectionError::Variables { .. }));
            assert_eq!(warning.site, expected_site);
            let actual_selections: Vec<_> = report
                .projections
                .iter()
                .map(|projection| projection.selected.clone())
                .collect();
            assert_eq!(actual_selections, expected_selections);
            assert_valid_projections(&guards, &constraints, &report.projections, sample_limit);
        }
    }

    #[test]
    fn known_valid_regions_respect_zero_and_small_sample_budgets() {
        let guard_sources = [r#"true"#, r#"cache_ready"#, r#"remote_ready"#];
        let guards: Vec<_> = guard_sources
            .iter()
            .map(|source| parse(source).unwrap())
            .collect();
        let constraints = [];
        // Cases pair a sample limit with the expected number of retained projections.
        let cases = [(0, 0), (1, 1)];
        for (sample_limit, expected_count) in cases {
            let budget = CfgBudget {
                variants: 2,
                samples: sample_limit,
                ..CfgBudget::default()
            };
            let report = project(guards.iter(), &constraints, budget);
            let warning = report.warning.as_ref().expect("sampled coverage warning");
            let expected_site = ProjectionSite::Guard(2);
            assert!(matches!(warning.error, ProjectionError::Variants { .. }));
            assert_eq!(warning.site, expected_site);
            assert_eq!(report.projections.len(), expected_count);
            assert_valid_projections(&guards, &constraints, &report.projections, sample_limit);
        }
    }

    #[test]
    fn zero_variant_budget_forces_sampled_fallback() {
        let budget = CfgBudget {
            variants: 0,
            samples: 1,
            ..CfgBudget::default()
        };
        let expected_max_samples = budget.samples;
        let guards = [Predicate::always()];
        let constraints = [];
        let report = project(guards.iter(), &constraints, budget);
        let warning = report.warning.as_ref().expect("sampled coverage warning");
        let expected_site = ProjectionSite::Guard(0);
        assert!(matches!(warning.error, ProjectionError::Variants { .. }));
        assert_eq!(warning.site, expected_site);
        let expected_selections = vec![vec![0]];
        let actual_selections: Vec<_> = report
            .projections
            .iter()
            .map(|projection| projection.selected.clone())
            .collect();
        assert_eq!(actual_selections, expected_selections);
        assert_valid_projections(
            &guards,
            &constraints,
            &report.projections,
            expected_max_samples,
        );
    }

    #[test]
    fn empty_combinators_and_invalid_predicates() {
        let empty_atoms = [];
        let assignment = 0;
        // Cases pair an empty Boolean combinator with its expected truth value.
        let valid_cases = [("all()", true), ("any()", false)];
        for (source, expected_match) in valid_cases {
            let predicate = parse(source).unwrap();
            assert_eq!(predicate.matches(&empty_atoms, assignment), expected_match);
        }
        // These cfg predicates are malformed or outside the supported syntax.
        let invalid_cases = [
            "not()",
            "not(unix, windows)",
            "all(42)",
            "feature = 42",
            r#"feature = concat!("a")"#,
            "unknown(unix)",
            "some::flag",
        ];
        for source in invalid_cases {
            assert!(parse(source).is_err(), "unexpectedly accepted {source}");
        }
    }
}
