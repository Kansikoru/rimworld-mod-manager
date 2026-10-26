//! "Find the broken mod" (F8): a halving search over the load order.
//!
//! The user runs the game with part of the mods and says whether the problem is still
//! there. What is always true while searching: the problem shows up with
//! `base + everything still under suspicion` (plus what those need), so every "yes"
//! narrows the suspects to the half that was on.
//!
//! - First the game runs with the base only (Core, DLC, Harmony, frameworks...). If the
//!   problem is there already, the cause is not among the suspects.
//! - A half is always enabled together with the mods it requires, so a test never fails
//!   just because a dependency is off.
//! - "No" for one half → the other half is tried. "No" for both → two mods break only
//!   together: one half stays on while the other is searched, then the other way round.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestKind {
    /// Only the mods that are never touched.
    BaseOnly,
    /// One half of the suspects.
    First,
    /// The other half, after "no" for the first one.
    Second,
}

/// One game launch: which suspects are on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Test {
    pub kind: TestKind,
    /// Suspects that are on in this launch.
    pub part: Vec<String>,
    /// The rest of the suspects.
    pub other: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    /// The mod that causes the problem; two or more = they only break together
    /// (or need each other and could not be split).
    Found(Vec<String>),
    /// The problem is there with the base only: not caused by the suspects.
    InBase,
}

/// Everything that changes with an answer (kept for "step back").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage {
    /// Mods one of which is (part of) the cause, in load order.
    pub candidates: Vec<String>,
    /// Found members of a group that breaks only together.
    pub found: Vec<String>,
    /// Groups still to search (they stay on meanwhile).
    pub pending: Vec<Vec<String>>,
    pub test: Option<Test>,
    pub outcome: Option<Outcome>,
    /// Answers given so far (= game launches).
    pub answers: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bisect {
    /// The load order at the start (lowercase IDs); restored at the end.
    pub original: Vec<String>,
    /// Always on: the kept mods and everything they require, in load order.
    pub base: Vec<String>,
    /// Requirements between the mods of `original`: ID -> IDs it needs.
    pub requires: BTreeMap<String, Vec<String>>,
    pub stage: Stage,
    pub history: Vec<Stage>,
    /// The current test is written to ModsConfig.xml.
    #[serde(default)]
    pub applied: bool,
}

impl Bisect {
    /// `keep`: mods that are never turned off. `requires`: what each mod needs (only
    /// mods of `original` matter; others are ignored).
    pub fn new(original: &[String], keep: &[String], requires: &BTreeMap<String, Vec<String>>) -> Self {
        let mut order: Vec<String> = Vec::with_capacity(original.len());
        for id in original {
            let id = id.to_lowercase();
            if !order.contains(&id) {
                order.push(id);
            }
        }
        let known: HashSet<&String> = order.iter().collect();
        let requires: BTreeMap<String, Vec<String>> = requires
            .iter()
            .map(|(k, v)| (k.to_lowercase(), v.iter().map(|d| d.to_lowercase()).collect::<Vec<_>>()))
            .filter(|(k, _)| known.contains(k))
            .map(|(k, v)| {
                let v: Vec<String> = v.into_iter().filter(|d| known.contains(d) && *d != k).collect();
                (k, v)
            })
            .filter(|(_, v)| !v.is_empty())
            .collect();

        let mut search = Self {
            original: order,
            base: Vec::new(),
            requires,
            stage: Stage {
                candidates: Vec::new(),
                found: Vec::new(),
                pending: Vec::new(),
                test: None,
                outcome: None,
                answers: 0,
            },
            history: Vec::new(),
            applied: false,
        };
        let keep_lower: Vec<String> = keep.iter().map(|k| k.to_lowercase()).collect();
        let base = search.closure(keep_lower.iter().filter(|k| search.original.contains(k)));
        search.base = search.ordered(&base);
        search.stage.candidates = search.original.iter().filter(|id| !base.contains(*id)).cloned().collect();
        if search.stage.candidates.is_empty() {
            search.stage.outcome = Some(Outcome::InBase);
        } else {
            search.stage.test = Some(Test { kind: TestKind::BaseOnly, part: Vec::new(), other: Vec::new() });
        }
        search
    }

    /// `ids` plus everything they require, directly or not.
    pub fn closure<'a>(&self, ids: impl IntoIterator<Item = &'a String>) -> HashSet<String> {
        let mut out: HashSet<String> = HashSet::new();
        let mut stack: Vec<String> = ids.into_iter().cloned().collect();
        while let Some(id) = stack.pop() {
            if out.insert(id.clone()) {
                if let Some(deps) = self.requires.get(&id) {
                    stack.extend(deps.iter().filter(|d| !out.contains(*d)).cloned());
                }
            }
        }
        out
    }

    /// The IDs of `set` in load order.
    fn ordered(&self, set: &HashSet<String>) -> Vec<String> {
        self.original.iter().filter(|id| set.contains(*id)).cloned().collect()
    }

    /// The load order for the current test (what to write to ModsConfig.xml).
    pub fn enabled(&self) -> Vec<String> {
        let Some(test) = &self.stage.test else {
            return self.original.clone();
        };
        let stage = &self.stage;
        let on = self.closure(
            self.base
                .iter()
                .chain(&stage.found)
                .chain(stage.pending.iter().flatten())
                .chain(&test.part),
        );
        self.ordered(&on)
    }

    /// The suspects that are on in the current test.
    pub fn tested_suspects(&self) -> &[String] {
        self.stage.test.as_ref().map(|t| t.part.as_slice()).unwrap_or(&[])
    }

    pub fn outcome(&self) -> Option<&Outcome> {
        self.stage.outcome.as_ref()
    }

    /// Mods still under suspicion.
    pub fn suspects_left(&self) -> usize {
        self.stage.candidates.len() + self.stage.pending.iter().map(Vec::len).sum::<usize>()
    }

    /// Two or more mods break only together: the search goes on for the rest of the group.
    pub fn searching_group(&self) -> bool {
        !self.stage.found.is_empty() || !self.stage.pending.is_empty()
    }

    /// About how many launches are left (at least; a "no" can add one).
    pub fn launches_left(&self) -> usize {
        if self.stage.outcome.is_some() {
            return 0;
        }
        let halvings = |n: usize| if n < 2 { 0 } else { (usize::BITS - (n - 1).leading_zeros()) as usize };
        let base = usize::from(matches!(self.stage.test.as_ref().map(|t| t.kind), Some(TestKind::BaseOnly)));
        base + halvings(self.stage.candidates.len()) + self.stage.pending.iter().map(|p| halvings(p.len())).sum::<usize>()
    }

    /// The user's answer for the current test: is the problem still there?
    pub fn answer(&mut self, problem: bool) {
        let Some(test) = self.stage.test.clone() else { return };
        self.history.push(self.stage.clone());
        self.stage.answers += 1;
        self.applied = false;
        match (test.kind, problem) {
            (TestKind::BaseOnly, true) => {
                self.stage.test = None;
                self.stage.outcome = Some(Outcome::InBase);
            }
            (TestKind::BaseOnly, false) => self.settle(),
            (_, true) => {
                self.stage.candidates = test.part;
                self.settle();
            }
            (TestKind::First, false) => {
                self.stage.test = Some(Test { kind: TestKind::Second, part: test.other, other: test.part });
            }
            (TestKind::Second, false) => {
                // Neither half alone: a mod from each half is needed. Keep this half on
                // and search the other one first.
                self.stage.pending.push(test.part);
                self.stage.candidates = test.other;
                self.settle();
            }
        }
    }

    /// Undoes the last answer.
    pub fn step_back(&mut self) -> bool {
        match self.history.pop() {
            Some(prev) => {
                self.stage = prev;
                self.applied = false;
                true
            }
            None => false,
        }
    }

    /// After the suspects changed: record a found mod, move on to a waiting group, or
    /// prepare the next test.
    fn settle(&mut self) {
        loop {
            if self.stage.candidates.len() == 1 {
                let id = self.stage.candidates.remove(0);
                self.stage.found.push(id);
            }
            if self.stage.candidates.is_empty() {
                if let Some(next) = self.stage.pending.pop() {
                    self.stage.candidates = next;
                    continue;
                }
                self.stage.test = None;
                self.stage.outcome = Some(Outcome::Found(self.stage.found.clone()));
                return;
            }
            match self.split(&self.stage.candidates) {
                Some((part, other)) => {
                    self.stage.test = Some(Test { kind: TestKind::First, part, other });
                    return;
                }
                None => {
                    // They all need each other: they can only be judged together.
                    let group: Vec<String> = std::mem::take(&mut self.stage.candidates);
                    self.stage.found.extend(group);
                }
            }
        }
    }

    /// Splits the suspects into two non-empty parts, the first closed under requirements
    /// (so turning it on brings no other suspect along) and as close to half as possible.
    fn split(&self, candidates: &[String]) -> Option<(Vec<String>, Vec<String>)> {
        if candidates.len() < 2 {
            return None;
        }
        let in_part = |on: &HashSet<String>| -> Vec<String> { candidates.iter().filter(|id| on.contains(*id)).cloned().collect() };
        let total = candidates.len();
        let consider = |best: &mut Option<Vec<String>>, part: Vec<String>| {
            let score = |n: usize| (2 * n).abs_diff(total);
            if !part.is_empty() && part.len() < total && best.as_ref().is_none_or(|b| score(part.len()) < score(b.len())) {
                *best = Some(part);
            }
        };
        let mut best: Option<Vec<String>> = None;

        // Prefixes in load order: requirements usually load earlier, so these split well.
        let mut on: HashSet<String> = HashSet::new();
        for id in &candidates[..total - 1] {
            on.extend(self.closure(std::iter::once(id)));
            let part = in_part(&on);
            if part.len() == total {
                break;
            }
            consider(&mut best, part);
        }
        // Unusual orders: single mods with what they need.
        if best.is_none() {
            for id in candidates {
                consider(&mut best, in_part(&self.closure(std::iter::once(id))));
            }
        }
        let part = best?;
        let part_set: HashSet<&String> = part.iter().collect();
        let other: Vec<String> = candidates.iter().filter(|id| !part_set.contains(id)).cloned().collect();
        Some((part, other))
    }

    /// Mods of the load order that need any of `ids`, directly or not (they stop working
    /// when `ids` are turned off).
    pub fn dependents_of(&self, ids: &[String]) -> Vec<String> {
        let targets: HashSet<String> = ids.iter().map(|i| i.to_lowercase()).collect();
        self.original
            .iter()
            .filter(|id| !targets.contains(*id))
            .filter(|id| {
                let needs = self.closure(std::iter::once(*id));
                needs.iter().any(|n| targets.contains(n))
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("mod.{i:02}")).collect()
    }

    fn reqs(pairs: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect())).collect()
    }

    /// Answers until the search ends; every tested load order must include what its mods need.
    fn run(search: &mut Bisect, problem: impl Fn(&[String]) -> bool) -> Outcome {
        for _ in 0..200 {
            if let Some(outcome) = search.outcome() {
                return outcome.clone();
            }
            let on = search.enabled();
            let set: HashSet<&String> = on.iter().collect();
            for id in &on {
                for dep in search.requires.get(id).into_iter().flatten() {
                    assert!(set.contains(dep), "{id} is on without {dep}");
                }
            }
            search.answer(problem(&on));
        }
        panic!("the search did not end");
    }

    #[test]
    fn finds_one_broken_mod_wherever_it_is() {
        let all = ids(40);
        for culprit in &all {
            let mut search = Bisect::new(&all, &[], &BTreeMap::new());
            let outcome = run(&mut search, |on| on.contains(culprit));
            assert_eq!(outcome, Outcome::Found(vec![culprit.clone()]));
            // Base check + at most two launches per halving (log2 40 → 6).
            assert!(search.stage.answers <= 1 + 2 * 6, "{} launches", search.stage.answers);
        }
    }

    #[test]
    fn halving_needs_about_log2_launches_when_the_first_half_is_guilty() {
        let all = ids(64);
        let culprit = all[0].clone();
        let mut search = Bisect::new(&all, &[], &BTreeMap::new());
        assert_eq!(search.launches_left(), 1 + 6);
        run(&mut search, |on| on.contains(&culprit));
        assert_eq!(search.stage.answers, 1 + 6);
    }

    #[test]
    fn kept_mods_and_their_requirements_stay_on() {
        let all = ids(10);
        let requires = reqs(&[("mod.00", &["mod.05"])]);
        let search = Bisect::new(&all, &["MOD.00".into()], &requires);
        assert_eq!(search.base, ["mod.00", "mod.05"]);
        assert!(!search.stage.candidates.contains(&"mod.05".to_string()));
        assert_eq!(search.enabled(), ["mod.00", "mod.05"], "the first launch is the base only");
    }

    #[test]
    fn problem_with_base_only_is_not_blamed_on_a_suspect() {
        let all = ids(10);
        let mut search = Bisect::new(&all, &["mod.03".into()], &BTreeMap::new());
        let outcome = run(&mut search, |on| on.contains(&"mod.03".to_string()));
        assert_eq!(outcome, Outcome::InBase);
        assert_eq!(search.stage.answers, 1);
    }

    #[test]
    fn finds_a_mod_that_others_depend_on_and_one_that_depends() {
        let all = ids(20);
        // Every fourth mod needs the one three places before it; mod.19 needs mod.01.
        let mut pairs: Vec<(String, Vec<String>)> =
            (4..20).step_by(4).map(|i| (format!("mod.{i:02}"), vec![format!("mod.{:02}", i - 3)])).collect();
        pairs.push(("mod.19".into(), vec!["mod.01".into()]));
        let requires: BTreeMap<String, Vec<String>> = pairs.into_iter().collect();
        for culprit in &all {
            let mut search = Bisect::new(&all, &[], &requires);
            let outcome = run(&mut search, |on| on.contains(culprit));
            assert_eq!(outcome, Outcome::Found(vec![culprit.clone()]), "culprit {culprit}");
        }
    }

    #[test]
    fn finds_two_mods_that_break_only_together() {
        let all = ids(32);
        for (x, y) in [(3, 27), (0, 31), (10, 12), (15, 16)] {
            let (x, y) = (all[x].clone(), all[y].clone());
            let mut search = Bisect::new(&all, &[], &BTreeMap::new());
            let outcome = run(&mut search, |on| on.contains(&x) && on.contains(&y));
            let Outcome::Found(mut found) = outcome else { panic!("not found") };
            found.sort();
            assert_eq!(found, [x, y]);
        }
    }

    #[test]
    fn mods_that_need_each_other_are_reported_together() {
        let all = ids(8);
        let requires = reqs(&[("mod.02", &["mod.06"]), ("mod.06", &["mod.02"])]);
        let mut search = Bisect::new(&all, &[], &requires);
        let outcome = run(&mut search, |on| on.contains(&"mod.06".to_string()));
        let Outcome::Found(mut found) = outcome else { panic!("not found") };
        found.sort();
        assert_eq!(found, ["mod.02", "mod.06"]);
    }

    #[test]
    fn step_back_undoes_an_answer() {
        let all = ids(16);
        let mut search = Bisect::new(&all, &[], &BTreeMap::new());
        search.answer(false); // base is fine
        let before = search.stage.clone();
        search.answer(true);
        assert_ne!(search.stage, before);
        assert!(search.step_back());
        assert_eq!(search.stage, before);
        assert!(search.step_back());
        assert!(!search.step_back(), "nothing left to undo");
    }

    #[test]
    fn survives_saving_and_loading() {
        let all = ids(12);
        let mut search = Bisect::new(&all, &["mod.00".into()], &reqs(&[("mod.04", &["mod.01"])]));
        search.answer(false);
        search.applied = true;
        let json = serde_json::to_string(&search).unwrap();
        let back: Bisect = serde_json::from_str(&json).unwrap();
        assert_eq!(back, search);
    }

    #[test]
    fn dependents_are_listed_for_turning_off() {
        let all = ids(6);
        let search = Bisect::new(&all, &[], &reqs(&[("mod.03", &["mod.01"]), ("mod.05", &["mod.03"])]));
        assert_eq!(search.dependents_of(&["mod.01".into()]), ["mod.03", "mod.05"]);
        assert!(search.dependents_of(&["mod.05".into()]).is_empty());
    }
}
