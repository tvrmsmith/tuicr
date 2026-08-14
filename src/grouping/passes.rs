//! The heuristic passes, and the resolver that turns their competing claims
//! into the strict partition `docs/GROUPING.md` requires.
//!
//! Everything here is **name-keyed and unordered**: a pass knows only which
//! files belong together and what to call the group. Opaque identity and
//! reading order are put on by [`super::Grouping`], the one place a presented
//! grouping is built.

use std::collections::{BTreeMap, BTreeSet};

use super::RunnerUp;
use super::changeset::{ChangeKind, ChangedFile, Changeset};

/// A pass's claim on a file: the group name it filed the file under, which pass
/// filed it, and — on close calls only — the group it nearly went to instead.
///
/// Derived state, recomputed on every regroup. `pass` and `runner_up` are
/// debug-only and never persisted (`docs/GROUPS_CONTRACT.md`).
#[derive(Debug, Clone)]
pub struct PassClaim {
    pub path: String,
    pub group: String,
    pub pass: &'static str,
    pub runner_up: Option<RunnerUp>,
}

#[derive(Debug, Clone, Copy)]
pub struct GroupingConfig {
    /// A filename token carried by at least this share of the changeset names
    /// the repo, not a concern, and is dropped as a cluster key.
    pub ubiquity_threshold: f64,
    /// Fewest files a token cluster may claim before it is just noise.
    pub min_cluster: usize,
    /// Whether the containing directory's tokens count as concern evidence.
    /// On where the tree is the concern tree, and harmless where it is not —
    /// but only once directory tokens are subject to the ubiquity stoplist too.
    pub use_dir_tokens: bool,
    /// Extra weight per added test file carrying a key (GROUPING.md rule 7).
    pub test_burst_boost: f64,
    /// Tie-break: a test file follows its production file (rule 4) even when a
    /// token cluster claims it. False lets the cluster keep it.
    pub tests_follow_production: bool,
    /// Whether files no cluster claimed join their most similar neighbour's
    /// group instead of falling through to a directory bucket.
    pub absorb_leftovers: bool,
    /// Fewest shared signals before absorption is more than a guess.
    pub absorb_min_similarity: f64,
}

impl Default for GroupingConfig {
    fn default() -> Self {
        Self {
            ubiquity_threshold: 0.25,
            min_cluster: 3,
            use_dir_tokens: true,
            test_burst_boost: 0.5,
            tests_follow_production: true,
            absorb_leftovers: true,
            absorb_min_similarity: 2.0,
        }
    }
}

/// CI wiring, which reads as one concern however it is scattered. Deliberately
/// *only* directories: the filename list this pass used to carry (`package.json`,
/// `Cargo.toml`, …) was guarded to the repo root, so it was dead in the monorepo
/// shape that has the most build config, and unguarding it is worth one file on
/// fixture 2 and nothing to F1. Grown to `*.csproj` it would actively harm —
/// fixture 2's 18 project files are one *concern* group by hand, not a config
/// group. Nested build config is left to token and directory evidence.
const CONFIG_DIRS: &[&str] = &[".github/", ".circleci/", ".husky/"];

/// Every file of the changeset filed under exactly one group name.
///
/// Total by construction: `directory_fallback_pass` claims unconditionally, so
/// nothing survives it unassigned (`docs/TOTAL_COVERAGE.md`).
pub fn assign(changeset: &Changeset, config: GroupingConfig) -> Vec<PassClaim> {
    let files: Vec<&ChangedFile> = changeset.files.iter().collect();
    let mut assigned: BTreeMap<&str, PassClaim> = BTreeMap::new();

    // Pass order is priority order: the earlier pass keeps a contested file.
    mechanical_pass(&files, &mut assigned);
    config_ci_pass(&files, &mut assigned);
    whole_change_docs_pass(&files, &mut assigned);
    let clusters = token_cluster_pass(&files, config, &mut assigned);
    test_pairing_pass(&files, config, &clusters, &mut assigned);
    if config.absorb_leftovers {
        absorb_leftovers_pass(&files, config, &mut assigned);
    }
    // No rename pass: git reports a rename as one entry keyed by the new path,
    // so a rename's two halves are already one file and GROUPING.md rule 11 is
    // satisfied by construction. See `a_rename_is_one_file_in_the_changeset`.
    directory_fallback_pass(&files, &mut assigned);

    assigned.into_values().collect()
}

fn claim<'a>(
    assigned: &mut BTreeMap<&'a str, PassClaim>,
    file: &'a ChangedFile,
    group: &str,
    pass: &'static str,
) {
    assigned
        .entry(file.path.as_str())
        .or_insert_with(|| PassClaim {
            path: file.path.clone(),
            group: group.to_string(),
            pass,
            runner_up: None,
        });
}

fn mechanical_pass<'a>(files: &[&'a ChangedFile], assigned: &mut BTreeMap<&'a str, PassClaim>) {
    for file in files {
        if file.is_mechanical() {
            claim(assigned, file, "mechanical", "mechanical");
        }
    }
}

fn config_ci_pass<'a>(files: &[&'a ChangedFile], assigned: &mut BTreeMap<&'a str, PassClaim>) {
    for file in files {
        if CONFIG_DIRS.iter().any(|dir| file.path.starts_with(dir)) {
            claim(assigned, file, "config-ci", "config-ci");
        }
    }
}

/// A doc covering the whole change gets its own group; a doc sitting beside the
/// code it documents is left to the cluster pass (GROUPING.md rule 9).
fn whole_change_docs_pass<'a>(
    files: &[&'a ChangedFile],
    assigned: &mut BTreeMap<&'a str, PassClaim>,
) {
    for file in files {
        let is_markdown = file.file_name().ends_with(".md");
        let whole_change_scope = !file.path.contains('/') || file.path.starts_with("docs/");
        if is_markdown && whole_change_scope {
            claim(assigned, file, "docs", "docs");
        }
    }
}

#[derive(Debug, Clone)]
pub struct Cluster {
    pub key: String,
    pub score: f64,
}

/// The load-bearing pass: distinctive filename tokens, with tokens the whole
/// changeset shares dropped so the engine cannot cheat on repo vocabulary.
fn token_cluster_pass<'a>(
    files: &[&'a ChangedFile],
    config: GroupingConfig,
    assigned: &mut BTreeMap<&'a str, PassClaim>,
) -> Vec<Cluster> {
    let keys_per_file: Vec<BTreeSet<String>> = files
        .iter()
        .map(|file| candidate_keys(file, config))
        .collect();

    let ubiquitous = ubiquitous_tokens(files, config);

    let mut carriers: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, keys) in keys_per_file.iter().enumerate() {
        for key in keys {
            // Ubiquity is judged on single tokens only: `github` names the repo,
            // but `github-project` still names a concern inside it.
            if !key.contains('-') && ubiquitous.contains(key) {
                continue;
            }
            carriers.entry(key.clone()).or_default().push(index);
        }
    }

    let mut ranked: Vec<Cluster> = carriers
        .iter()
        .filter(|(_, indices)| indices.len() >= config.min_cluster)
        .map(|(key, indices)| {
            let added_tests = indices
                .iter()
                .filter(|&&i| files[i].is_test() && files[i].kind == ChangeKind::Added)
                .count() as f64;
            let specificity = if key.contains('-') { 1.6 } else { 1.0 };
            Cluster {
                key: key.clone(),
                score: indices.len() as f64 * specificity + added_tests * config.test_burst_boost,
            }
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap()
            .then(b.key.len().cmp(&a.key.len()))
            .then(a.key.cmp(&b.key))
    });

    let mut formed: Vec<Cluster> = Vec::new();
    for cluster in &ranked {
        let takers: Vec<usize> = carriers[&cluster.key]
            .iter()
            .copied()
            .filter(|&i| !assigned.contains_key(files[i].path.as_str()))
            .collect();
        if takers.len() < config.min_cluster {
            continue;
        }
        for index in takers {
            claim(assigned, files[index], &cluster.key, "token-cluster");
        }
        formed.push(cluster.clone());
    }

    record_runners_up(files, &keys_per_file, &formed, assigned);
    formed
}

/// The cluster keys a file carries: its name tokens, adjacent token pairs, and
/// — when `use_dir_tokens` — its directory's tokens.
///
/// Public for the fixture harness's baselines, which must ask the same question
/// of a file as the pass does or they are not baselines for this engine.
pub fn candidate_keys(file: &ChangedFile, config: GroupingConfig) -> BTreeSet<String> {
    let tokens = file.name_tokens();
    let mut keys: BTreeSet<String> = tokens.iter().cloned().collect();
    for pair in tokens.windows(2) {
        keys.insert(format!("{}-{}", pair[0], pair[1]));
    }
    if config.use_dir_tokens {
        keys.extend(file.dir_tokens());
    }
    keys
}

/// Whatever counts as evidence must also be subject to the stoplist: a
/// directory token admitted as a cluster key and exempt from ubiquity would be
/// a free pass for `src`, `app` or the repo's own name.
fn ubiquitous_tokens(files: &[&ChangedFile], config: GroupingConfig) -> BTreeSet<String> {
    let mut document_frequency: BTreeMap<String, usize> = BTreeMap::new();
    for file in files {
        let mut tokens: BTreeSet<String> = file.name_tokens().into_iter().collect();
        if config.use_dir_tokens {
            tokens.extend(file.dir_tokens());
        }
        for token in tokens {
            *document_frequency.entry(token).or_default() += 1;
        }
    }
    let limit = config.ubiquity_threshold * files.len() as f64;
    document_frequency
        .into_iter()
        .filter(|(_, count)| *count as f64 >= limit)
        .map(|(token, _)| token)
        .collect()
}

/// GROUPING.md's close-call annotation: the group that came second, and why.
fn record_runners_up<'a>(
    files: &[&'a ChangedFile],
    keys_per_file: &[BTreeSet<String>],
    formed: &[Cluster],
    assigned: &mut BTreeMap<&'a str, PassClaim>,
) {
    let scores: BTreeMap<&str, f64> = formed.iter().map(|c| (c.key.as_str(), c.score)).collect();
    for (index, keys) in keys_per_file.iter().enumerate() {
        let Some(assignment) = assigned.get_mut(files[index].path.as_str()) else {
            continue;
        };
        if assignment.pass != "token-cluster" {
            continue;
        }
        let chosen_score = scores
            .get(assignment.group.as_str())
            .copied()
            .unwrap_or(0.0);
        let runner_up = keys
            .iter()
            .filter(|key| key.as_str() != assignment.group)
            .filter_map(|key| scores.get(key.as_str()).map(|score| (key, *score)))
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        if let Some((key, score)) = runner_up
            && score >= 0.75 * chosen_score
        {
            assignment.runner_up = Some(RunnerUp {
                group: key.clone(),
                reason: format!(
                    "filename also carries `{key}`; `{}` scored {:.1} against {:.1}",
                    assignment.group, chosen_score, score
                ),
            });
        }
    }
}

/// Rule 4: a test file lives in its production file's group. Runs after
/// clustering so there are groups to join, but under
/// `tests_follow_production` it overrides a cluster that already took the test.
fn test_pairing_pass<'a>(
    files: &[&'a ChangedFile],
    config: GroupingConfig,
    clusters: &[Cluster],
    assigned: &mut BTreeMap<&'a str, PassClaim>,
) {
    let cluster_scores: BTreeMap<&str, f64> =
        clusters.iter().map(|c| (c.key.as_str(), c.score)).collect();

    for file in files {
        if !file.is_test() {
            continue;
        }
        let Some(partner) = production_partner(file, files) else {
            continue;
        };
        let Some(partner_group) = assigned.get(partner.path.as_str()).map(|a| a.group.clone())
        else {
            continue;
        };

        match assigned.get_mut(file.path.as_str()) {
            None => claim(assigned, file, &partner_group, "test-pairing"),
            Some(existing) if existing.group == partner_group => {}
            Some(existing)
                if config.tests_follow_production && existing.pass == "token-cluster" =>
            {
                let displaced = existing.group.clone();
                let cluster_score = cluster_scores
                    .get(displaced.as_str())
                    .copied()
                    .unwrap_or(0.0);
                existing.group = partner_group;
                existing.pass = "test-pairing";
                existing.runner_up = Some(RunnerUp {
                    group: displaced.clone(),
                    reason: format!(
                        "follows production file {} (rule 4) over cluster `{displaced}` (score {:.1})",
                        partner.path, cluster_score
                    ),
                });
            }
            Some(_) => {}
        }
    }
}

/// The production file a test covers: same stem in the same directory, else the
/// longest same-directory stem the test name extends at a `-` boundary.
fn production_partner<'a>(
    test: &ChangedFile,
    files: &[&'a ChangedFile],
) -> Option<&'a ChangedFile> {
    let stem = test.stem();
    let candidates: Vec<&&ChangedFile> = files
        .iter()
        .filter(|other| !other.is_test() && other.dir() == test.dir())
        .collect();

    if let Some(exact) = candidates.iter().find(|other| other.stem() == stem) {
        return Some(exact);
    }
    candidates
        .iter()
        .filter(|other| stem.starts_with(&format!("{}-", other.stem())))
        .max_by_key(|other| other.stem().len())
        .map(|other| **other)
}

/// Files no cluster claimed join the group of their most similar already-placed
/// file, rather than fragmenting into directory buckets. Scored against a
/// snapshot so absorption order cannot chain one weak guess onto another.
fn absorb_leftovers_pass<'a>(
    files: &[&'a ChangedFile],
    config: GroupingConfig,
    assigned: &mut BTreeMap<&'a str, PassClaim>,
) {
    let placed: Vec<(&ChangedFile, String)> = files
        .iter()
        .filter_map(|file| {
            assigned
                .get(file.path.as_str())
                .map(|a| (*file, a.group.clone()))
        })
        .collect();

    let absorbed: Vec<(&ChangedFile, String, Option<RunnerUp>)> = files
        .iter()
        .filter(|file| !assigned.contains_key(file.path.as_str()))
        .filter_map(|file| {
            let mut by_group: BTreeMap<&str, f64> = BTreeMap::new();
            for (other, group) in &placed {
                let similarity = similarity(file, other);
                let best = by_group.entry(group.as_str()).or_insert(0.0);
                *best = best.max(similarity);
            }
            let mut ranked: Vec<(&str, f64)> = by_group.into_iter().collect();
            ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(b.0)));
            let (best_group, best_score) = ranked.first().copied()?;
            if best_score < config.absorb_min_similarity {
                return None;
            }
            let runner_up = ranked.get(1).filter(|(_, score)| *score >= 0.75 * best_score).map(
                |(group, score)| RunnerUp {
                    group: group.to_string(),
                    reason: format!(
                        "no cluster claimed it; nearest `{best_group}` scored {best_score:.1} against {score:.1}"
                    ),
                },
            );
            Some((*file, best_group.to_string(), runner_up))
        })
        .collect();

    for (file, group, runner_up) in absorbed {
        assigned.insert(
            file.path.as_str(),
            PassClaim {
                path: file.path.clone(),
                group,
                pass: "absorb",
                runner_up,
            },
        );
    }
}

/// Shared filename tokens, with directory proximity as a weaker tiebreaker —
/// enough to place a stray, never enough to outvote a token cluster.
///
/// Public for the same reason as [`candidate_keys`]: the agglomerative baseline
/// clusters over this exact similarity, which is what makes it a control for
/// the greedy pass rather than a different experiment.
pub fn similarity(file: &ChangedFile, other: &ChangedFile) -> f64 {
    let own: BTreeSet<String> = file.name_tokens().into_iter().collect();
    let theirs: BTreeSet<String> = other.name_tokens().into_iter().collect();
    let shared = own.intersection(&theirs).count() as f64;
    let proximity = if file.dir() == other.dir() {
        1.0
    } else if file.top_level_dir() == other.top_level_dir() {
        0.25
    } else {
        0.0
    };
    shared + proximity
}

/// Last resort for files no concern claimed. GROUPING.md rule 6 calls this a
/// smell, so the report counts what lands here.
fn directory_fallback_pass<'a>(
    files: &[&'a ChangedFile],
    assigned: &mut BTreeMap<&'a str, PassClaim>,
) {
    for file in files {
        let group = format!("{}{}", super::order::FALLBACK_PREFIX, file.dir());
        claim(assigned, file, &group, "directory-fallback");
    }
}
