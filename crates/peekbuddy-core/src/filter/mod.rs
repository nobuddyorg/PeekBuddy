//! The filter engine (#40): rules compiled once, evaluated without
//! allocation on the capture thread.
//!
//! A [`Rule`] says what to match (a glob, a regex, extensions, event kinds or
//! flag bits), whether it includes or excludes, and what an exclusion does:
//! [`Action::HideInView`] (the default: the event is stored and hidden) or
//! [`Action::NeverRecord`] (a user-chosen, counted loss: the event is dropped
//! at ingest). The last matching rule wins; an event no rule matches is
//! recorded and visible.
//!
//! [`FilterSet::compile`] builds, per case mode and scope, fully built DFAs
//! (no lazy cache, so matching never allocates). [`FilterSet::eval`] folds
//! the candidate name with the ordinal table of #39 into a [`Scratch`] and
//! asks the DFAs which patterns match. The delivered and the resolved name
//! are both tested.
//!
//! The glob dialect is in [`glob`]; presets are in [`presets`].

pub mod glob;
pub mod presets;

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use regex_automata::dfa::{Automaton, OverlappingState, dense};
use regex_automata::nfa::thompson;
use regex_automata::util::syntax;
use regex_automata::{Input, MatchKind};

use crate::event::{Event, Flags, Kind, MAX_NAME_UNITS, RootIdx};
use crate::name::{CaseMode, WideStr, fold_upper, fold_upper_into, max_utf8_len, to_lossy_utf8};

pub use glob::GlobError;

/// The most rules one set holds.
pub const MAX_RULES: usize = 100_000;

/// Size limit for one compiled NFA, in bytes.
const NFA_SIZE_LIMIT: usize = 2 * 1024 * 1024;
/// Size limit for one compiled DFA, in bytes. DFAs live as long as the
/// engine, so this is part of its memory budget (F8).
const DFA_SIZE_LIMIT: usize = 2 * 1024 * 1024;
/// Size limit for the working memory of building one DFA, in bytes.
const DETERMINIZE_SIZE_LIMIT: usize = 8 * 1024 * 1024;

/// Where a rule applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every watch root.
    Global,
    /// One watch root.
    Watch(RootIdx),
}

/// A set of event kinds, by kind class (the payload of
/// [`Kind::Overflow`] and [`Kind::WatchStateChanged`] is ignored).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KindSet(u32);

impl KindSet {
    /// The empty set.
    pub const EMPTY: Self = Self(0);

    /// The set of `kinds`.
    #[must_use]
    pub fn of(kinds: &[Kind]) -> Self {
        Self(kinds.iter().fold(0, |bits, &kind| bits | Self::bit(kind)))
    }

    /// Whether `kind` is in the set.
    #[must_use]
    pub const fn contains(self, kind: Kind) -> bool {
        self.0 & Self::bit(kind) != 0
    }

    /// Whether the set is empty.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    const fn bit(kind: Kind) -> u32 {
        1 << kind.to_wire().0
    }
}

/// What a rule matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    /// A gitignore-style glob (see [`glob`]).
    Glob(String),
    /// A regex searched anywhere in the path (linear time; size limited).
    Regex(String),
    /// File extensions, with or without the dot (`tmp` or `.tmp`).
    Ext(Vec<String>),
    /// Event kinds.
    Kinds(KindSet),
    /// Matches when any of these flag bits is set on the event.
    Flags(Flags),
}

/// Whether a match excludes or re-includes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Polarity {
    /// A match shows the event again.
    Include,
    /// A match applies the rule's [`Action`].
    Exclude,
}

/// What an exclusion does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Action {
    /// Store the event and hide it in views (the default, F3).
    #[default]
    HideInView,
    /// Drop the event at ingest and count it. Only ever user-chosen.
    NeverRecord,
}

/// One filter rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    /// Where the rule applies.
    pub scope: Scope,
    /// What it matches.
    pub pattern: Pattern,
    /// Include or exclude.
    pub polarity: Polarity,
    /// What an exclusion does.
    pub action: Action,
}

impl Rule {
    /// A global exclude rule that hides matches in views.
    #[must_use]
    pub const fn exclude(pattern: Pattern) -> Self {
        Self {
            scope: Scope::Global,
            pattern,
            polarity: Polarity::Exclude,
            action: Action::HideInView,
        }
    }

    /// A global rule that re-includes matches.
    #[must_use]
    pub const fn include(pattern: Pattern) -> Self {
        Self {
            scope: Scope::Global,
            pattern,
            polarity: Polarity::Include,
            action: Action::HideInView,
        }
    }

    /// The same rule, applying to one watch root only.
    #[must_use]
    pub const fn in_watch(mut self, root: RootIdx) -> Self {
        self.scope = Scope::Watch(root);
        self
    }

    /// The same rule, dropping matches at ingest.
    #[must_use]
    pub const fn never_record(mut self) -> Self {
        self.action = Action::NeverRecord;
        self
    }
}

/// Why a rule list does not compile. Every variant names the rule index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterError {
    /// More than [`MAX_RULES`] rules.
    TooManyRules,
    /// A glob does not parse.
    Glob {
        /// The rule index.
        rule: usize,
        /// The parse error.
        error: GlobError,
    },
    /// A regex does not compile or exceeds the size limits.
    Regex {
        /// The rule index.
        rule: usize,
        /// A readable message.
        message: String,
    },
    /// An extension list is empty or holds an invalid entry.
    Ext {
        /// The rule index.
        rule: usize,
    },
    /// A kind set is empty.
    NoKinds {
        /// The rule index.
        rule: usize,
    },
    /// A flag mask is empty.
    NoFlags {
        /// The rule index.
        rule: usize,
    },
    /// An include rule asks for [`Action::NeverRecord`].
    IncludeNeverRecord {
        /// The rule index.
        rule: usize,
    },
    /// A glob starting with `!` sits in an include rule.
    NegatedInclude {
        /// The rule index.
        rule: usize,
    },
    /// The rules of one scope together exceed the matcher size limits.
    TooComplex {
        /// The scope.
        scope: Scope,
    },
}

impl fmt::Display for FilterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyRules => write!(f, "more than {MAX_RULES} rules"),
            Self::Glob { rule, error } => write!(f, "rule {rule}: {error}"),
            Self::Regex { rule, message } => write!(f, "rule {rule}: {message}"),
            Self::Ext { rule } => write!(f, "rule {rule}: invalid extension list"),
            Self::NoKinds { rule } => write!(f, "rule {rule}: no event kinds"),
            Self::NoFlags { rule } => write!(f, "rule {rule}: no flags"),
            Self::IncludeNeverRecord { rule } => {
                write!(f, "rule {rule}: an include rule cannot drop events")
            }
            Self::NegatedInclude { rule } => {
                write!(f, "rule {rule}: `!` in an include rule")
            }
            Self::TooComplex { scope } => {
                write!(f, "the rules of {scope:?} are too complex to compile")
            }
        }
    }
}

impl std::error::Error for FilterError {}

/// Case rules for compilation: a default and per-root overrides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileOptions {
    /// The rule for roots without an override.
    pub default_case: CaseMode,
    /// Roots whose directories are case-sensitive (or not, explicitly).
    pub overrides: Vec<(RootIdx, CaseMode)>,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            default_case: CaseMode::Insensitive,
            overrides: Vec::new(),
        }
    }
}

/// The result of evaluating one event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    /// Whether the event is stored. `false` only for a matching
    /// [`Action::NeverRecord`] rule.
    pub record: bool,
    /// Whether views show the event.
    pub visible: bool,
    /// The index of the deciding rule; `None` when no rule matched.
    pub matched_rule: Option<usize>,
}

/// Totals over every evaluated event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
    /// Events evaluated.
    pub seen: u64,
    /// Events stored.
    pub recorded: u64,
    /// Events dropped by `NeverRecord` rules.
    pub dropped: u64,
}

/// Per-thread working memory for [`FilterSet::eval`], allocated once.
#[derive(Debug)]
pub struct Scratch {
    units: Box<[u16]>,
    bytes: Box<[u8]>,
}

impl Scratch {
    /// Working memory for the longest name.
    #[must_use]
    pub fn new() -> Self {
        Self {
            units: vec![0; MAX_NAME_UNITS].into_boxed_slice(),
            bytes: vec![0; max_utf8_len(MAX_NAME_UNITS)].into_boxed_slice(),
        }
    }
}

impl Default for Scratch {
    fn default() -> Self {
        Self::new()
    }
}

type Dfa = dense::DFA<Vec<u32>>;

#[derive(Debug, Clone, Copy)]
struct CompiledRule {
    polarity: Polarity,
    action: Action,
}

#[derive(Debug, Clone, Copy)]
struct GlobEntry {
    rule: usize,
    dir_only: bool,
}

#[derive(Debug)]
enum Simple {
    Ext(Vec<Vec<u16>>),
    Kinds(KindSet),
    Flags(Flags),
}

#[derive(Debug, Default)]
struct ScopeMatcher {
    globs: Option<Dfa>,
    glob_entries: Vec<GlobEntry>,
    regexes: Option<Dfa>,
    regex_rules: Vec<usize>,
    simple: Vec<(usize, Simple)>,
}

#[derive(Debug, Default)]
struct Variant {
    global: ScopeMatcher,
    roots: Vec<(RootIdx, ScopeMatcher)>,
}

/// A compiled rule list.
#[derive(Debug)]
pub struct FilterSet {
    rules: Vec<CompiledRule>,
    options: CompileOptions,
    insensitive: Option<Variant>,
    sensitive: Option<Variant>,
    seen: AtomicU64,
    recorded: AtomicU64,
    dropped: AtomicU64,
    dropped_by_rule: Vec<AtomicU64>,
}

/// A rule after validation, before it is compiled per mode.
enum Prepared {
    Glob(String),
    Regex(String),
    Simple(PreparedSimple),
}

enum PreparedSimple {
    Ext(Vec<String>),
    Kinds(KindSet),
    Flags(Flags),
}

fn prepare(index: usize, rule: &Rule) -> Result<(Prepared, Polarity), FilterError> {
    let (prepared, polarity) = match &rule.pattern {
        Pattern::Glob(text) => {
            let parsed = glob::parse(text, CaseMode::Sensitive)
                .map_err(|error| FilterError::Glob { rule: index, error })?;
            if parsed.negated && rule.polarity == Polarity::Include {
                return Err(FilterError::NegatedInclude { rule: index });
            }
            let polarity = if parsed.negated {
                Polarity::Include
            } else {
                rule.polarity
            };
            (Prepared::Glob(text.clone()), polarity)
        }
        Pattern::Regex(text) => (Prepared::Regex(text.clone()), rule.polarity),
        Pattern::Ext(list) => {
            let mut cleaned = Vec::with_capacity(list.len());
            for entry in list {
                let entry = entry.strip_prefix('.').unwrap_or(entry);
                if entry.is_empty() || entry.contains(['.', '/', '\\', '\0']) {
                    return Err(FilterError::Ext { rule: index });
                }
                cleaned.push(entry.to_owned());
            }
            if cleaned.is_empty() {
                return Err(FilterError::Ext { rule: index });
            }
            (
                Prepared::Simple(PreparedSimple::Ext(cleaned)),
                rule.polarity,
            )
        }
        Pattern::Kinds(set) => {
            if set.is_empty() {
                return Err(FilterError::NoKinds { rule: index });
            }
            (Prepared::Simple(PreparedSimple::Kinds(*set)), rule.polarity)
        }
        Pattern::Flags(flags) => {
            if flags.bits() == 0 {
                return Err(FilterError::NoFlags { rule: index });
            }
            (
                Prepared::Simple(PreparedSimple::Flags(*flags)),
                rule.polarity,
            )
        }
    };
    if polarity == Polarity::Include && rule.action == Action::NeverRecord {
        return Err(FilterError::IncludeNeverRecord { rule: index });
    }
    Ok((prepared, polarity))
}

/// Patterns of one scope, before the DFAs are built.
#[derive(Default)]
struct ScopeAcc {
    globs: Vec<String>,
    glob_entries: Vec<GlobEntry>,
    regexes: Vec<String>,
    regex_rules: Vec<usize>,
    simple: Vec<(usize, Simple)>,
}

/// `bytewise` lets a pattern match arbitrary bytes (the glob translation
/// uses byte classes to keep its DFAs small); user regexes stay UTF-8.
fn syntax_config(case_insensitive: bool, bytewise: bool) -> syntax::Config {
    syntax::Config::new()
        .case_insensitive(case_insensitive)
        .unicode(true)
        .utf8(!bytewise)
}

fn thompson_config(bytewise: bool) -> thompson::Config {
    thompson::Config::new()
        .nfa_size_limit(Some(NFA_SIZE_LIMIT))
        .utf8(!bytewise)
}

/// Checks that one pattern compiles within the NFA limit.
fn check_pattern(source: &str, case_insensitive: bool, bytewise: bool) -> Result<(), String> {
    thompson::Compiler::new()
        .configure(thompson_config(bytewise))
        .syntax(syntax_config(case_insensitive, bytewise))
        .build(source)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn build_dfa(patterns: &[String], case_insensitive: bool, bytewise: bool) -> Option<Dfa> {
    dense::Builder::new()
        .configure(
            dense::Config::new()
                .match_kind(MatchKind::All)
                .dfa_size_limit(Some(DFA_SIZE_LIMIT))
                .determinize_size_limit(Some(DETERMINIZE_SIZE_LIMIT))
                .minimize(false),
        )
        .syntax(syntax_config(case_insensitive, bytewise))
        .thompson(thompson_config(bytewise))
        .build_many(patterns)
        .ok()
}

fn fold_units(text: &str, mode: CaseMode) -> Vec<u16> {
    text.encode_utf16()
        .map(|unit| {
            if mode == CaseMode::Insensitive {
                fold_upper(unit)
            } else {
                unit
            }
        })
        .collect()
}

impl FilterSet {
    /// Compiles `rules` with every root case-insensitive (the NTFS default).
    ///
    /// # Errors
    ///
    /// A [`FilterError`] naming the first rule that does not compile.
    pub fn compile(rules: &[Rule]) -> Result<Self, FilterError> {
        Self::compile_with(rules, &CompileOptions::default())
    }

    /// Compiles `rules` with the case rules of `options`.
    ///
    /// # Errors
    ///
    /// A [`FilterError`] naming the first rule that does not compile.
    pub fn compile_with(rules: &[Rule], options: &CompileOptions) -> Result<Self, FilterError> {
        if rules.len() > MAX_RULES {
            return Err(FilterError::TooManyRules);
        }
        let mut prepared = Vec::with_capacity(rules.len());
        let mut compiled = Vec::with_capacity(rules.len());
        for (index, rule) in rules.iter().enumerate() {
            let (p, polarity) = prepare(index, rule)?;
            prepared.push(p);
            compiled.push(CompiledRule {
                polarity,
                action: rule.action,
            });
        }
        let mut needs_insensitive = options.default_case == CaseMode::Insensitive;
        let mut needs_sensitive = options.default_case == CaseMode::Sensitive;
        for &(_, mode) in &options.overrides {
            needs_insensitive |= mode == CaseMode::Insensitive;
            needs_sensitive |= mode == CaseMode::Sensitive;
        }
        let build = |mode| Self::build_variant(rules, &prepared, mode);
        let insensitive = if needs_insensitive {
            Some(build(CaseMode::Insensitive)?)
        } else {
            None
        };
        let sensitive = if needs_sensitive {
            Some(build(CaseMode::Sensitive)?)
        } else {
            None
        };
        Ok(Self {
            dropped_by_rule: (0..rules.len()).map(|_| AtomicU64::new(0)).collect(),
            rules: compiled,
            options: options.clone(),
            insensitive,
            sensitive,
            seen: AtomicU64::new(0),
            recorded: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        })
    }

    fn build_variant(
        rules: &[Rule],
        prepared: &[Prepared],
        mode: CaseMode,
    ) -> Result<Variant, FilterError> {
        let insensitive = mode == CaseMode::Insensitive;
        let mut global = ScopeAcc::default();
        let mut roots: BTreeMap<u16, ScopeAcc> = BTreeMap::new();
        for (index, (rule, prep)) in rules.iter().zip(prepared).enumerate() {
            let acc = match rule.scope {
                Scope::Global => &mut global,
                Scope::Watch(root) => roots.entry(root.0).or_default(),
            };
            match prep {
                Prepared::Glob(pattern) => {
                    let parsed = glob::parse(pattern, mode)
                        .map_err(|error| FilterError::Glob { rule: index, error })?;
                    check_pattern(&parsed.regex, false, true).map_err(|message| {
                        FilterError::Regex {
                            rule: index,
                            message,
                        }
                    })?;
                    acc.globs.push(parsed.regex);
                    acc.glob_entries.push(GlobEntry {
                        rule: index,
                        dir_only: parsed.dir_only,
                    });
                }
                Prepared::Regex(source) => {
                    check_pattern(source, insensitive, false).map_err(|message| {
                        FilterError::Regex {
                            rule: index,
                            message,
                        }
                    })?;
                    acc.regexes.push(source.clone());
                    acc.regex_rules.push(index);
                }
                Prepared::Simple(simple) => {
                    let simple = match simple {
                        PreparedSimple::Ext(list) => {
                            Simple::Ext(list.iter().map(|e| fold_units(e, mode)).collect())
                        }
                        PreparedSimple::Kinds(set) => Simple::Kinds(*set),
                        PreparedSimple::Flags(flags) => Simple::Flags(*flags),
                    };
                    acc.simple.push((index, simple));
                }
            }
        }
        let finish = |scope, acc: ScopeAcc| -> Result<ScopeMatcher, FilterError> {
            let globs = if acc.globs.is_empty() {
                None
            } else {
                Some(build_dfa(&acc.globs, false, true).ok_or(FilterError::TooComplex { scope })?)
            };
            let regexes = if acc.regexes.is_empty() {
                None
            } else {
                Some(
                    build_dfa(&acc.regexes, insensitive, false)
                        .ok_or(FilterError::TooComplex { scope })?,
                )
            };
            Ok(ScopeMatcher {
                globs,
                glob_entries: acc.glob_entries,
                regexes,
                regex_rules: acc.regex_rules,
                simple: acc.simple,
            })
        };
        let global = finish(Scope::Global, global)?;
        let mut built = Vec::with_capacity(roots.len());
        for (root, acc) in roots {
            built.push((RootIdx(root), finish(Scope::Watch(RootIdx(root)), acc)?));
        }
        Ok(Variant {
            global,
            roots: built,
        })
    }

    /// The number of rules.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.rules.len()
    }

    /// Events dropped by rule `rule` (0 for an unknown index).
    #[must_use]
    pub fn dropped_by_rule(&self, rule: usize) -> u64 {
        self.dropped_by_rule
            .get(rule)
            .map_or(0, |c| c.load(Ordering::Relaxed))
    }

    /// Bytes held by the compiled DFAs (the filter's share of the engine's
    /// memory budget, F8).
    #[must_use]
    pub fn memory_usage(&self) -> usize {
        let matcher = |m: &ScopeMatcher| {
            m.globs.as_ref().map_or(0, Dfa::memory_usage)
                + m.regexes.as_ref().map_or(0, Dfa::memory_usage)
        };
        [&self.insensitive, &self.sensitive]
            .into_iter()
            .flatten()
            .map(|v| matcher(&v.global) + v.roots.iter().map(|(_, m)| matcher(m)).sum::<usize>())
            .sum()
    }

    /// Totals over every evaluated event.
    #[must_use]
    pub fn stats(&self) -> Stats {
        Stats {
            seen: self.seen.load(Ordering::Relaxed),
            recorded: self.recorded.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
        }
    }

    fn mode_of(&self, root: RootIdx) -> CaseMode {
        self.options
            .overrides
            .iter()
            .rev()
            .find(|(r, _)| *r == root)
            .map_or(self.options.default_case, |&(_, mode)| mode)
    }

    /// Evaluates one event whose delivered name is `name`.
    pub fn eval(&self, ev: &Event, name: WideStr<'_>, scratch: &mut Scratch) -> Verdict {
        self.eval_with_resolved(ev, name, None, scratch)
    }

    /// Evaluates one event, testing the delivered `name` and the resolved
    /// long name (#39) when there is one. A name longer than
    /// [`MAX_NAME_UNITS`] is not tested.
    pub fn eval_with_resolved(
        &self,
        ev: &Event,
        name: WideStr<'_>,
        resolved: Option<WideStr<'_>>,
        scratch: &mut Scratch,
    ) -> Verdict {
        let mode = self.mode_of(ev.root);
        let variant = match mode {
            CaseMode::Insensitive => self.insensitive.as_ref(),
            CaseMode::Sensitive => self.sensitive.as_ref(),
        };
        let mut best: Option<usize> = None;
        if let Some(variant) = variant {
            for candidate in [Some(name), resolved].into_iter().flatten() {
                match_candidate(variant, mode, ev, candidate, scratch, &mut best);
            }
            for matcher in [Some(&variant.global), variant.root(ev.root)]
                .into_iter()
                .flatten()
            {
                match_event_only(matcher, ev, &mut best);
            }
        }
        let verdict = match best.and_then(|i| self.rules.get(i).map(|rule| (i, rule))) {
            None => Verdict {
                record: true,
                visible: true,
                matched_rule: None,
            },
            Some((i, rule)) => match (rule.polarity, rule.action) {
                (Polarity::Include, _) => Verdict {
                    record: true,
                    visible: true,
                    matched_rule: Some(i),
                },
                (Polarity::Exclude, Action::HideInView) => Verdict {
                    record: true,
                    visible: false,
                    matched_rule: Some(i),
                },
                (Polarity::Exclude, Action::NeverRecord) => Verdict {
                    record: false,
                    visible: false,
                    matched_rule: Some(i),
                },
            },
        };
        self.seen.fetch_add(1, Ordering::Relaxed);
        if verdict.record {
            self.recorded.fetch_add(1, Ordering::Relaxed);
        } else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            if let Some(counter) = verdict
                .matched_rule
                .and_then(|i| self.dropped_by_rule.get(i))
            {
                counter.fetch_add(1, Ordering::Relaxed);
            }
        }
        verdict
    }
}

impl Variant {
    fn root(&self, root: RootIdx) -> Option<&ScopeMatcher> {
        self.roots
            .binary_search_by_key(&root.0, |(r, _)| r.0)
            .ok()
            .and_then(|i| self.roots.get(i))
            .map(|(_, matcher)| matcher)
    }
}

fn raise(best: &mut Option<usize>, rule: usize) {
    if best.is_none_or(|b| rule > b) {
        *best = Some(rule);
    }
}

/// Calls `found` with the pattern index and end offset of every match of
/// `dfa` in `haystack`; stops when `found` returns `false`.
fn each_match(dfa: &Dfa, haystack: &[u8], mut found: impl FnMut(usize, usize) -> bool) {
    let input = Input::new(haystack);
    let mut state = OverlappingState::start();
    while dfa.try_search_overlapping_fwd(&input, &mut state).is_ok() {
        match state.get_match() {
            Some(m) if !found(m.pattern().as_usize(), m.offset()) => break,
            Some(_) => {}
            None => break,
        }
    }
}

/// The extension of the last component: what follows its last `.`, unless
/// the dot starts the component or ends it.
fn extension(units: &[u16]) -> Option<&[u16]> {
    let is_sep = |&u: &u16| u == u16::from(b'/') || u == u16::from(b'\\');
    let leaf_start = units.iter().rposition(is_sep).map_or(0, |i| i + 1);
    let leaf = units.get(leaf_start..)?;
    let dot = leaf.iter().rposition(|&u| u == u16::from(b'.'))?;
    let ext = leaf.get(dot + 1..)?;
    (dot > 0 && !ext.is_empty()).then_some(ext)
}

/// Whether a glob prefix match ending at `end` counts: it must be non-empty
/// and end at the end of the path or before a separator. A match that ends
/// the path is the path itself, which a directory-only glob accepts only for
/// a directory; one that ends before a separator is an ancestor, always a
/// directory.
fn glob_match_ok(haystack: &[u8], end: usize, dir_only: bool, is_dir: bool) -> bool {
    if end == 0 {
        return false;
    }
    match haystack.get(end) {
        None => !dir_only || is_dir,
        Some(&byte) => byte == b'/' || byte == b'\\',
    }
}

fn match_candidate(
    variant: &Variant,
    mode: CaseMode,
    ev: &Event,
    candidate: WideStr<'_>,
    scratch: &mut Scratch,
    best: &mut Option<usize>,
) {
    let len = candidate.len();
    let Some(units) = scratch.units.get_mut(..len) else {
        return;
    };
    match mode {
        CaseMode::Insensitive => {
            if fold_upper_into(candidate.units(), units).is_none() {
                return;
            }
        }
        CaseMode::Sensitive => units.copy_from_slice(candidate.units()),
    }
    let units: &[u16] = units;
    let Some(text) = to_lossy_utf8(WideStr::new(units), &mut scratch.bytes) else {
        return;
    };
    let haystack = text.text.as_bytes();
    for matcher in [Some(&variant.global), variant.root(ev.root)]
        .into_iter()
        .flatten()
    {
        if let Some(dfa) = &matcher.globs {
            each_match(dfa, haystack, |pattern, end| {
                if let Some(entry) = matcher.glob_entries.get(pattern)
                    && glob_match_ok(haystack, end, entry.dir_only, ev.is_dir)
                {
                    raise(best, entry.rule);
                }
                true
            });
        }
        if let Some(dfa) = &matcher.regexes {
            each_match(dfa, haystack, |pattern, _| {
                if let Some(&rule) = matcher.regex_rules.get(pattern) {
                    raise(best, rule);
                }
                // Any match is enough for this rule, but other patterns of
                // the DFA may still match.
                true
            });
        }
        for (rule, simple) in &matcher.simple {
            if let Simple::Ext(list) = simple
                && extension(units).is_some_and(|ext| list.iter().any(|e| e.as_slice() == ext))
            {
                raise(best, *rule);
            }
        }
    }
}

fn match_event_only(matcher: &ScopeMatcher, ev: &Event, best: &mut Option<usize>) {
    for (rule, simple) in &matcher.simple {
        let hit = match simple {
            Simple::Kinds(set) => set.contains(ev.kind),
            Simple::Flags(flags) => ev.flags.bits() & flags.bits() != 0,
            Simple::Ext(_) => false,
        };
        if hit {
            raise(best, *rule);
        }
    }
}

#[cfg(test)]
mod tests;
