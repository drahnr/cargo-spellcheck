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
    /// An owner's fragment projections checked against exhaustive cfg evaluation.
    struct Case {
        /// Scenario name included in assertion failures.
        name: &'static str,
        /// One predicate per source fragment; its slice position is the fragment's identity.
        doc_guards: &'static [&'static str],
        /// Predicates that must all hold for the owner to exist; an empty slice allows any cfg.
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

/// Check complete witnesses, owner constraints, known atom relations, and exact fragment selection.
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
        let release_assignment = Vec::from_iter(assignment.iter().filter_map(|(atom, enabled)| {
            if let Atom::Version(version) = atom {
                Some((version, *enabled))
            } else {
                None
            }
        }));
        // Check every ordered pair independently of the adjacent-implication implementation.
        for (older, older_enabled) in &release_assignment {
            for (newer, newer_enabled) in &release_assignment {
                if older <= newer && *newer_enabled {
                    assert!(*older_enabled, "inconsistent release witness: {source}");
                }
            }
        }
        let platform_value = |atom: &Atom| match atom {
            Atom::Flag(name) if matches!(name.as_str(), "unix" | "windows") => Some(name == "unix"),
            Atom::Value(key, family)
                if key == "target_family" && matches!(family.as_str(), "unix" | "windows") =>
            {
                Some(family == "unix")
            }
            _ => None,
        };
        for (left, left_enabled) in &assignment {
            for (right, right_enabled) in &assignment {
                if let (Some(left), Some(right)) = (platform_value(left), platform_value(right)) {
                    if left == right {
                        assert_eq!(left_enabled, right_enabled, "inconsistent alias: {source}");
                    } else {
                        assert!(
                            !left_enabled || !right_enabled,
                            "incompatible platform witness: {source}"
                        );
                    }
                }
            }
        }
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
    // These owner constraints produce no valid configuration in the bounded sample.
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
    // Cases pair a sample limit with the expected selected source-fragment indices.
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
fn packed_assignment_preserves_bits_across_word_boundaries() {
    let enabled_bits = [0, 63, 64, 129];
    let mut assignment = vec![false; 130];
    for index in enabled_bits {
        assignment[index] = true;
    }
    // Words contain bits 0/63, bit 64, and bit 129 respectively.
    let expected_words = vec![1u64 | (1u64 << 63), 1, 2];
    assert_eq!(packed_assignment(&assignment), expected_words);
}

#[test]
fn zero_sampling_budget_does_not_prepare_an_atom_index() {
    let source = "all(cache, transport)";
    let guards = [parse(source).unwrap()];
    let guard_refs = Vec::from_iter(guards.iter());
    let constraints = [];
    let expected_atoms = 0;
    let collector = ProjectionCollector::new(&guard_refs, &constraints, 0);
    assert_eq!(collector.atoms.len(), expected_atoms);
}

#[test]
fn retained_assignments_do_not_spend_the_sample_budget_twice() {
    /// Guards and expected fragment identities for the lost-alternative regression.
    struct Case {
        guards: &'static [&'static str],
        expected_selections: &'static [&'static [usize]],
    }
    let cases = [
        Case {
            guards: &[r#"feature = "telemetry""#],
            expected_selections: &[&[0]],
        },
        Case {
            guards: &["true", r#"feature = "telemetry""#],
            expected_selections: &[&[0], &[0, 1]],
        },
    ];
    let budget = CfgBudget {
        variants: 1,
        samples: 2,
        ..CfgBudget::default()
    };
    let constraints = [];
    for Case {
        guards: sources,
        expected_selections,
    } in cases
    {
        let guards = Vec::from_iter(sources.iter().map(|source| parse(source).unwrap()));
        let report = project(&guards, &constraints, budget);
        // The retained all-false witness must leave room for the all-true alternative.
        assert!(report.warning.is_some());
        let actual = std::collections::BTreeSet::from_iter(
            report.projections.iter().map(|p| p.selected.clone()),
        );
        let expected = std::collections::BTreeSet::from_iter(
            expected_selections.iter().map(|indices| indices.to_vec()),
        );
        assert_eq!(actual, expected);
        assert_valid_projections(&guards, &constraints, &report.projections, budget.samples);
    }
}

#[test]
fn sampling_exhausts_small_domains_without_charging_repeated_candidates() {
    let sources = ["cache", "tls", "metrics", "trace"];
    let guards = Vec::from_iter(sources.iter().map(|source| parse(source).unwrap()));
    let guard_refs = Vec::from_iter(guards.iter());
    // Cases pair owner constraints with the expected number of nonempty valid selections.
    let cases: [(&[&str], usize); 2] = [(&[], 15), (&["any(cache, tls)"], 12)];
    let expected_assignments = 16;
    for (constraint_sources, expected_projections) in cases {
        let constraints = Vec::from_iter(
            constraint_sources
                .iter()
                .map(|source| parse(source).unwrap()),
        );
        let constraint_refs = Vec::from_iter(constraints.iter());
        let mut collector =
            ProjectionCollector::new(&guard_refs, &constraint_refs, expected_assignments);
        // Seed the same endpoint the sampler starts with to exercise shared accounting.
        collector.record(vec![false; sources.len()]);
        collector.sample_remaining();
        assert_eq!(collector.assignments.len(), expected_assignments);
        assert_eq!(collector.projections.len(), expected_projections);
        assert_valid_projections(
            &guards,
            &constraints,
            &collector.projections,
            expected_assignments,
        );
    }
}

#[test]
fn bdd_task_budget_counts_nonterminal_work_even_for_a_constant_result() {
    let sources = ["transport", "not(transport)"];
    let predicates = sources.map(|source| parse(source).unwrap());
    let space = ConditionSpace::new(predicates.iter()).unwrap();
    let left = space.compile(&predicates[0]).unwrap();
    let right = space.compile(&predicates[1]).unwrap();
    // Cases pair the task limit with the expected truth value; None means budget exhaustion.
    let cases = [(0, None), (1, Some(false))];
    for (task_limit, expected_value) in cases {
        let budget = CfgBudget {
            bdd_tasks: task_limit,
            ..CfgBudget::default()
        };
        let actual = budget.combine(&left, &right, op_function::and).ok();
        let expected = expected_value.map(|value| {
            if value {
                space.variables.mk_true()
            } else {
                space.variables.mk_false()
            }
        });
        assert_eq!(actual, expected);
    }
}

#[test]
fn terminal_operations_obey_node_limits_without_requiring_any_tasks() {
    // Cases pair a constant predicate with the BDD representation's required node count.
    let cases = [("false", 1), ("true", 2)];
    for (source, required_nodes) in cases {
        let predicate = parse(source).unwrap();
        let space = ConditionSpace::new(std::iter::once(&predicate)).unwrap();
        let terminal = space.compile(&predicate).unwrap();
        for node_limit in 0..=required_nodes {
            let budget = CfgBudget {
                bdd_nodes: node_limit,
                bdd_tasks: 0,
                ..CfgBudget::default()
            };
            let actual = budget.combine(&terminal, &terminal, op_function::and).ok();
            let expected = (node_limit >= required_nodes).then(|| terminal.clone());
            assert_eq!(actual, expected, "{source}, node limit {node_limit}");
        }
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

/// The parser accepts the union of identifier spellings valid in supported editions/releases.
#[test]
fn cfg_identifiers_accept_edition_and_msrv_compatible_spellings() {
    // These bare names were checked with Rust 2015 or the Rust 1.88 compiler baseline.
    let sources = [
        "self", "super", "Self", "crate", "async", "await", "dyn", "try", "gen", "union",
        "default", "café", "all", "any", "not",
    ];
    for source in sources {
        let expected_atom = Atom::Flag(source.to_owned());
        let Predicate::Atom(actual_atom) = parse(source).unwrap() else {
            panic!("{source} must be an atomic cfg option");
        };
        assert_eq!(actual_atom, expected_atom);
    }
    // Cases pair a raw identifier with the same atom produced by its plain spelling.
    let raw_cases = [
        ("r#async", "async"),
        ("r#type", "type"),
        ("r#unsafe", "unsafe"),
        ("r#true", "true"),
        ("r#unix", "unix"),
    ];
    for (source, expected_name) in raw_cases {
        let expected_atom = Atom::Flag(expected_name.to_owned());
        let Predicate::Atom(actual_atom) = parse(source).unwrap() else {
            panic!("{source} must be an atomic cfg option");
        };
        assert_eq!(actual_atom, expected_atom);
    }
}

#[test]
fn cfg_identifiers_reject_nonlegacy_keywords_and_underscore() {
    for source in ["unsafe", "type", "fn", "mod", "_"] {
        let error = parse(source).expect_err("invalid cfg identifier must be rejected");
        assert_eq!(error.to_string(), "expected a cfg identifier", "{source}");
    }
}

/// Canonically equivalent identifiers share atoms while distinct string values remain distinct.
#[test]
fn unicode_cfg_identifiers_are_normalized_but_string_values_are_not() {
    let source = r#"any(all(café, not(café)), all(r#café = "one", not(café = "one")))"#;
    let predicate = parse(source).unwrap();
    let space = ConditionSpace::new(std::iter::once(&predicate)).unwrap();
    let expected_atoms = vec![
        Atom::Flag("café".to_owned()),
        Atom::Value("café".to_owned(), "one".to_owned()),
    ];
    assert_eq!(space.atoms, expected_atoms);
    assert!(space.compile(&predicate).unwrap().is_false());

    let value_source = r#"all(café = "café", not(café = "café"))"#;
    let predicate = parse(value_source).unwrap();
    let space = ConditionSpace::new(std::iter::once(&predicate)).unwrap();
    let expected_atoms = vec![
        Atom::Value("café".to_owned(), "café".to_owned()),
        Atom::Value("café".to_owned(), "café".to_owned()),
    ];
    assert_eq!(space.atoms, expected_atoms);
    assert!(!space.compile(&predicate).unwrap().is_false());
}

/// Witness names remain parseable across editions without inventing forbidden raw self paths.
#[test]
fn cfg_identifier_rendering_preserves_legacy_names_and_escapes_edition_keywords() {
    // Cases pair the normalized atom name with its expected cfg spelling.
    let cases = [
        ("self", "self"),
        ("super", "super"),
        ("Self", "Self"),
        ("crate", "crate"),
        ("async", "r#async"),
        ("gen", "r#gen"),
        ("unsafe", "r#unsafe"),
        ("true", "r#true"),
    ];
    for (name, expected_spelling) in cases {
        let atom = Atom::Flag(name.to_owned());
        assert_eq!(atom.as_predicate(), expected_spelling);
    }
}

/// String spellings are decoded by Rust literal syntax, not treated as distinct cfg values.
#[test]
fn cfg_option_strings_accept_raw_unicode_escapes_and_line_continuations() {
    /// Source spelling and the decoded key-value atom it must produce.
    struct Case {
        source: &'static str,
        expected_key: &'static str,
        expected_value: &'static str,
    }
    let cases = [
        Case {
            source: r#"feature = "one""#,
            expected_key: "feature",
            expected_value: "one",
        },
        Case {
            source: r###"r#feature = r#"one"#"###,
            expected_key: "feature",
            expected_value: "one",
        },
        Case {
            source: r#"feature = "\x6f\u{006e}e""#,
            expected_key: "feature",
            expected_value: "one",
        },
        Case {
            source: r#"path = "C:\\routes""#,
            expected_key: "path",
            expected_value: r#"C:\routes"#,
        },
        Case {
            source: r#"café = "naïve""#,
            expected_key: "café",
            expected_value: "naïve",
        },
        Case {
            source: r####"feature = r###"quoted "doc" # tag"###"####,
            expected_key: "feature",
            expected_value: r#"quoted "doc" # tag"#,
        },
        Case {
            source: r#"feature = "line\
    continuation""#,
            expected_key: "feature",
            expected_value: "linecontinuation",
        },
        Case {
            source: r#"async = "legacy""#,
            expected_key: "async",
            expected_value: "legacy",
        },
        Case {
            source: r#"self = "legacy""#,
            expected_key: "self",
            expected_value: "legacy",
        },
        Case {
            source: r#"r#true = "option""#,
            expected_key: "true",
            expected_value: "option",
        },
    ];
    for Case {
        source,
        expected_key,
        expected_value,
    } in cases
    {
        let expected_atom = Atom::Value(expected_key.to_owned(), expected_value.to_owned());
        let Predicate::Atom(actual_atom) = parse(source).unwrap() else {
            panic!("{source} must be a key-value cfg option");
        };
        assert_eq!(actual_atom, expected_atom, "{source}");
    }
}

/// Parenthesized combinators and compact targets retain their documented Boolean meaning.
#[test]
fn cfg_combinator_and_compact_target_spellings_share_the_expected_atoms() {
    // Cases pair accepted predicate syntax with an equivalent conventional cfg expression.
    let cases = [
        ("all()", "true"),
        ("any()", "false"),
        ("all(unix,)", "unix"),
        ("any(not(unix,), windows,)", "any(not(unix), windows)"),
        (
            r###"r#all(r#unix, not(r#feature = r#"one"#,),)"###,
            r#"all(unix, not(feature = "one"))"#,
        ),
        ("target()", "true"),
        (
            r#"target(os = "linux", arch = "x86_64",)"#,
            r#"all(target_os = "linux", target_arch = "x86_64")"#,
        ),
        (
            r###"target(r#feature = r#"sse2"#, family = "unix",)"###,
            r#"all(target_feature = "sse2", target_family = "unix")"#,
        ),
        (
            r#"all(target(os = "linux"), not(target_os = "linux"))"#,
            "false",
        ),
        (
            r#"target(target_os = "linux")"#,
            r#"target_target_os = "linux""#,
        ),
        (r#"not(version("1.88",),)"#, r#"not(version("1.88"))"#),
    ];
    for (source, expected_source) in cases {
        let original = parse(source).unwrap();
        let expected = parse(expected_source).unwrap();
        let space = ConditionSpace::new([&original, &expected].into_iter()).unwrap();
        let actual = space.compile(&original).unwrap();
        let expected = space.compile(&expected).unwrap();
        assert_eq!(actual, expected, "{source}");
    }
}

/// Nightly cfg(version) remains symbolic and accepts raw strings and its optional comma.
#[test]
fn cfg_version_spellings_preserve_one_symbolic_version_atom() {
    let expected_version = RustVersion([1, 88, 0]);
    let expected_atom = Atom::Version(expected_version);
    let expected_spelling = r#"version("1.88")"#;
    // Each source is a spelling of the same version predicate, not a host-version query.
    let sources = [
        r#"version("1.88")"#,
        r#"version("1.88",)"#,
        r###"r#version(r#"1.88"#,)"###,
    ];
    for source in sources {
        let Predicate::Atom(actual_atom) = parse(source).unwrap() else {
            panic!("{source} must be a version atom");
        };
        assert_eq!(actual_atom, expected_atom);
        assert_eq!(actual_atom.as_predicate(), expected_spelling);
    }
}

/// Build nested predicates without recursing in the test's source generator.
fn nested_cfg(operator: &str, depth: usize) -> String {
    format!(
        "{}unix{}",
        format!("{operator}(").repeat(depth),
        ")".repeat(depth)
    )
}

#[test]
fn cfg_predicate_depth_is_bounded_on_a_small_stack() {
    let expected_depth_limit = 32;
    let expected_error = "cfg predicate nesting limit exceeded (32)";
    let operators = ["not", "all", "any"];
    let stack_bytes = 2 * 1024 * 1024;
    std::thread::Builder::new()
        .stack_size(stack_bytes)
        .spawn(move || {
            for operator in operators {
                // This safe assertion must fail before the pre-fix aborting input is tried.
                let rejected_source = nested_cfg(operator, expected_depth_limit + 1);
                let error =
                    parse(&rejected_source).expect_err("excessive nesting must be rejected");
                assert_eq!(error.to_string(), expected_error, "{operator}");
                let deep_source = nested_cfg(operator, 1024);
                let error = parse(&deep_source).expect_err("deep nesting must not abort");
                assert_eq!(error.to_string(), expected_error, "{operator}");

                let accepted_depths = [3, expected_depth_limit - 1, expected_depth_limit];
                for depth in accepted_depths {
                    let source = nested_cfg(operator, depth);
                    let predicate = parse(&source).unwrap();
                    let cloned = predicate.clone();
                    let guards = [cloned];
                    let constraints = [];
                    let budget = CfgBudget::default();
                    let report = project(guards.iter(), &constraints, budget);
                    let expected_selections = vec![vec![0]];
                    let actual_selections = Vec::from_iter(
                        report
                            .projections
                            .iter()
                            .map(|projection| projection.selected.clone()),
                    );
                    assert!(report.warning.is_none(), "{operator}, depth {depth}");
                    assert_eq!(
                        actual_selections, expected_selections,
                        "{operator}, depth {depth}"
                    );
                    assert_valid_projections(
                        &guards,
                        &constraints,
                        &report.projections,
                        budget.samples,
                    );
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn cfg_versions_canonicalize_rust_release_spellings() {
    // Cases pair a rustc-accepted release spelling with its canonical symbolic predicate.
    let cases = [
        (r#"version("1.88")"#, r#"version("1.88")"#),
        (r#"version("1.88.0")"#, r#"version("1.88")"#),
        (r#"version("01.088.000")"#, r#"version("1.88")"#),
        (r#"version("+1.+88.+0")"#, r#"version("1.88")"#),
        (r###"r#version(r#"1.88.0"#,)"###, r#"version("1.88")"#),
        (r#"version("1.88.1")"#, r#"version("1.88.1")"#),
        (r#"version("01.088.001")"#, r#"version("1.88.1")"#),
        (r#"version("1.88.2")"#, r#"version("1.88.2")"#),
        (
            r#"version("65535.65535.65535")"#,
            r#"version("65535.65535.65535")"#,
        ),
    ];
    for (source, expected_source) in cases {
        let expected = parse(expected_source).unwrap();
        let predicate = parse(source).unwrap();
        let space = ConditionSpace::new([&predicate, &expected].into_iter()).unwrap();
        let expected_atom_count = 1;
        assert_eq!(space.atoms.len(), expected_atom_count, "{source}");
        let expected_spelling = expected_source;
        assert_eq!(space.atoms[0].as_predicate(), expected_spelling, "{source}");
    }
}

#[test]
fn cfg_versions_reject_unknown_rust_release_literals() {
    let expected_error = "expected a Rust release version with two or three u16 components";
    // Rust's cfg(version) warns for these unknown version strings, rather than using SemVer.
    let sources = [
        r#"version("")"#,
        r#"version("1")"#,
        r#"version("1.88.")"#,
        r#"version("1.88.0.1")"#,
        r#"version("1.88.0-nightly")"#,
        r#"version("1.88.0-dev")"#,
        r#"version("1.88.0+build")"#,
        r#"version(" 1.88")"#,
        r#"version("1.88 ")"#,
        r#"version("-1.88")"#,
        r#"version("1.65536")"#,
        r#"version("65536.0.0")"#,
        r#"version("1.88.65536")"#,
    ];
    for source in sources {
        let error = parse(source).expect_err("unknown release must produce an attribute warning");
        assert_eq!(error.to_string(), expected_error, "{source}");
    }
}

#[test]
fn cfg_version_projections_obey_release_order_including_fallback() {
    /// Explicit fragment selections for one symbolic compiler-release scenario.
    struct Case {
        /// Scenario included in assertion messages.
        name: &'static str,
        /// Guards in source-fragment order.
        sources: &'static [&'static str],
        /// Conditions required by the documentation owner.
        constraints: &'static [&'static str],
        /// All realizable, nonempty source-fragment selections.
        expected_selections: &'static [&'static [usize]],
    }
    let cases = [
        Case {
            name: "equivalent release spellings cannot split fragments",
            sources: &["true", r#"version("1.88")"#, r#"version("1.88.0")"#],
            constraints: &[],
            expected_selections: &[&[0], &[0, 1, 2]],
        },
        Case {
            name: "patch thresholds stay distinct while implying equivalent minor thresholds",
            sources: &[
                "true",
                r#"version("1.88.1")"#,
                r#"version("1.88")"#,
                r#"version("1.88.0")"#,
            ],
            constraints: &[],
            expected_selections: &[&[0], &[0, 2, 3], &[0, 1, 2, 3]],
        },
        Case {
            name: "minor release can be selected without its first patch release",
            sources: &["true", r#"version("1.88")"#, r#"version("1.88.1")"#],
            constraints: &[r#"version("1.88")"#, r#"not(version("1.88.1"))"#],
            expected_selections: &[&[0, 1]],
        },
        Case {
            name: "patch release cannot be selected without its minor threshold",
            sources: &["true"],
            constraints: &[r#"version("1.88.1")"#, r#"not(version("1.88"))"#],
            expected_selections: &[],
        },
        Case {
            name: "newer releases imply older releases",
            sources: &["true", r#"version("1.88")"#, r#"version("1.90")"#],
            constraints: &[],
            expected_selections: &[&[0], &[0, 1], &[0, 1, 2]],
        },
        Case {
            name: "numeric ordering is independent of atom insertion and lexicographic order",
            sources: &[
                "true",
                r#"version("1.10")"#,
                r#"version("1.9")"#,
                r#"version("1.10.2")"#,
            ],
            constraints: &[],
            expected_selections: &[&[0], &[0, 2], &[0, 1, 2], &[0, 1, 2, 3]],
        },
        Case {
            name: "owner constraints participate in release consistency",
            sources: &["true", r#"version("1.88")"#, r#"not(version("1.91"))"#],
            constraints: &[r#"version("1.90")"#],
            expected_selections: &[&[0, 1], &[0, 1, 2]],
        },
        Case {
            name: "contradictory release interval emits no fragments",
            sources: &["true"],
            constraints: &[r#"version("1.90")"#, r#"not(version("1.88"))"#],
            expected_selections: &[],
        },
        Case {
            name: "future major releases stay symbolic rather than host evaluated",
            sources: &["true", r#"version("2.0")"#, r#"version("1.65535.4")"#],
            constraints: &[],
            expected_selections: &[&[0], &[0, 2], &[0, 1, 2]],
        },
    ];
    // Each budget name identifies an exact path or a separately forced fallback path.
    let budgets = [
        ("exact BDD", CfgBudget::default()),
        (
            "variable fallback",
            CfgBudget {
                variables: 0,
                ..CfgBudget::default()
            },
        ),
        (
            "node fallback",
            CfgBudget {
                bdd_nodes: 0,
                ..CfgBudget::default()
            },
        ),
        (
            "task fallback",
            CfgBudget {
                bdd_tasks: 0,
                ..CfgBudget::default()
            },
        ),
        (
            "region fallback",
            CfgBudget {
                variants: 0,
                ..CfgBudget::default()
            },
        ),
        (
            "retained-region fallback",
            CfgBudget {
                variants: 1,
                ..CfgBudget::default()
            },
        ),
    ];
    for case in cases {
        let guards = Vec::from_iter(case.sources.iter().map(|source| parse(source).unwrap()));
        let constraints =
            Vec::from_iter(case.constraints.iter().map(|source| parse(source).unwrap()));
        let expected_selections = std::collections::BTreeSet::from_iter(
            case.expected_selections
                .iter()
                .map(|selection| selection.to_vec()),
        );
        for (budget_name, budget) in budgets {
            let budget = CfgBudget {
                samples: 16,
                ..budget
            };
            let report = project(guards.iter(), &constraints, budget);
            let actual_selections = std::collections::BTreeSet::from_iter(
                report
                    .projections
                    .iter()
                    .map(|projection| projection.selected.clone()),
            );
            assert_eq!(
                actual_selections, expected_selections,
                "{}: {budget_name}",
                case.name
            );
            // Impossible owners and single constrained selections can finish within
            // the zero/one-region budgets without needing fallback.
            let within_region_budget =
                budget.variants <= 1 && case.expected_selections.len() <= budget.variants;
            let expected_warning = budget_name != "exact BDD" && !within_region_budget;
            assert_eq!(
                report.warning.is_some(),
                expected_warning,
                "{}: {budget_name}",
                case.name
            );
            assert_valid_projections(&guards, &constraints, &report.projections, budget.samples);
        }
    }
}

#[test]
fn cfg_platform_relations_apply_to_exact_and_sampled_projections() {
    struct Case {
        name: &'static str,
        sources: &'static [&'static str],
        constraints: &'static [&'static str],
        expected: &'static [&'static [usize]],
    }
    let cases = [
        Case {
            name: "platform flags exclude each other without being exhaustive",
            sources: &["true", "unix", "windows", "all(unix, windows)"],
            constraints: &[],
            expected: &[&[0], &[0, 1], &[0, 2]],
        },
        Case {
            name: "aliases stay correlated across guards",
            sources: &[
                "true",
                "unix",
                r#"target_family = "unix""#,
                "windows",
                r#"target_family = "windows""#,
            ],
            constraints: &[],
            expected: &[&[0], &[0, 1, 2], &[0, 3, 4]],
        },
        Case {
            name: "raw and compact spellings retain the same relations",
            sources: &[
                "true",
                r#"target(family = "windows")"#,
                "r#windows",
                r#"r#target_family = "unix""#,
                "r#unix",
            ],
            constraints: &[],
            expected: &[&[0], &[0, 1, 2], &[0, 3, 4]],
        },
        Case {
            name: "incompatible flag constraints remove the owner",
            sources: &["true"],
            constraints: &["unix", "windows"],
            expected: &[],
        },
        Case {
            name: "family constraints exclude conflicting flags",
            sources: &["true", "windows"],
            constraints: &[r#"target_family = "unix""#],
            expected: &[&[0]],
        },
        Case {
            name: "incompatible family constraints remove the owner",
            sources: &["true"],
            constraints: &[r#"target_family = "unix""#, r#"target_family = "windows""#],
            expected: &[],
        },
        Case {
            name: "flag implies its family",
            sources: &["true"],
            constraints: &["unix", r#"not(target_family = "unix")"#],
            expected: &[],
        },
        Case {
            name: "family implies its flag",
            sources: &["true"],
            constraints: &[r#"target_family = "windows""#, "not(windows)"],
            expected: &[],
        },
        Case {
            name: "other target family values can coexist",
            sources: &[
                "true",
                r#"target_family = "unix""#,
                r#"target_family = "wasm""#,
            ],
            constraints: &[],
            expected: &[&[0], &[0, 1], &[0, 2], &[0, 1, 2]],
        },
        Case {
            name: "unrelated flags and values remain independent",
            sources: &["true", "custom", r#"custom = "unix""#],
            constraints: &[],
            expected: &[&[0], &[0, 1], &[0, 2], &[0, 1, 2]],
        },
    ];
    let budgets = [
        CfgBudget::default(),
        CfgBudget {
            variables: 0,
            ..CfgBudget::default()
        },
        CfgBudget {
            bdd_nodes: 0,
            ..CfgBudget::default()
        },
        CfgBudget {
            bdd_tasks: 0,
            ..CfgBudget::default()
        },
        CfgBudget {
            variants: 0,
            ..CfgBudget::default()
        },
        CfgBudget {
            variants: 1,
            ..CfgBudget::default()
        },
    ];
    for case in cases {
        let guards = Vec::from_iter(case.sources.iter().map(|source| parse(source).unwrap()));
        let constraints =
            Vec::from_iter(case.constraints.iter().map(|source| parse(source).unwrap()));
        let expected = std::collections::BTreeSet::from_iter(
            case.expected.iter().map(|selection| selection.to_vec()),
        );
        for budget in budgets {
            let report = project(guards.iter(), &constraints, budget);
            let actual = std::collections::BTreeSet::from_iter(
                report
                    .projections
                    .iter()
                    .map(|projection| projection.selected.clone()),
            );
            assert_eq!(actual, expected, "{}: {budget:?}", case.name);
            if budget == CfgBudget::default() {
                assert!(report.warning.is_none(), "{}", case.name);
            }
            assert_valid_projections(&guards, &constraints, &report.projections, budget.samples);
        }
    }
}

#[test]
fn cfg_version_projections_keep_conditional_fences_complete() {
    // Each entry pairs a fragment guard with its literal documentation text.
    let fragments = [
        ("true", "Release notes.\n"),
        (
            r#"version("1.88")"#,
            r#"```rust
let release = 88;
"#,
        ),
        (r#"version("1.88.0")"#, "```\n"),
        (
            r#"all(version("1.90"), not(version("1.88")))"#,
            "impossible release\n",
        ),
    ];
    let guards = Vec::from_iter(fragments.iter().map(|(source, _)| parse(source).unwrap()));
    let constraints = [];
    let budgets = [
        CfgBudget::default(),
        CfgBudget {
            variables: 0,
            ..CfgBudget::default()
        },
    ];
    let expected_documents = std::collections::BTreeSet::from([
        "Release notes.\n".to_owned(),
        r#"Release notes.
```rust
let release = 88;
```
"#
        .to_owned(),
    ]);
    for budget in budgets {
        let report = project(guards.iter(), &constraints, budget);
        let actual_documents =
            std::collections::BTreeSet::from_iter(report.projections.iter().map(|projection| {
                String::from_iter(projection.selected.iter().map(|index| fragments[*index].1))
            }));
        assert_eq!(actual_documents, expected_documents);
        assert_valid_projections(&guards, &constraints, &report.projections, budget.samples);
    }
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
        "all[unix]",
        "not{unix}",
        "unsafe",
        "all(42)",
        "feature = 42",
        r#"feature = concat!("a")"#,
        r#"feature = "one"suffix"#,
        r#"version("1.88", "1.90")"#,
        r#"version("1.88"suffix)"#,
        "version()",
        "version(42)",
        "target(unix)",
        "target(all())",
        "target(pointer_width = 64)",
        r#"target(os = "linux"suffix)"#,
        "_",
        "unknown(unix)",
        "some::flag",
    ];
    for source in invalid_cases {
        assert!(parse(source).is_err(), "unexpectedly accepted {source}");
    }
}
