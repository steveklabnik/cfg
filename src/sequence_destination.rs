use collections::range::RangeArgument;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

use history::{Action, RewriteSequence};
use rule_builder::RuleBuilder;
use rule_container::RuleContainer;
use sequence::{Separator, Sequence};
use sequence::Separator::{Trailing, Proper, Liberal};
use sequence_builder::SequenceRuleBuilder;
use symbol::{GrammarSymbol, SymbolSource};

/// Trait for storing sequence rules in containers, with potential rewrites.
pub trait SequenceDestination<H> {
    /// The type of symbols.
    type Symbol;
    /// Inserts a sequence rule.
    fn add_sequence(&mut self, seq: Sequence<H, Self::Symbol>);
}

pub struct SequencesToProductions<H, D> where
            H: RewriteSequence,
            D: RuleContainer {
    destination: D,
    stack: Vec<Sequence<H::Rewritten, D::Symbol>>,
    map: HashMap<PartialSequence<D::Symbol>, D::Symbol>,
}

// A key into a private map.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct PartialSequence<S> {
    rhs: S,
    start: u32,
    end: Option<u32>,
    separator: Separator<S>,
}

impl<'a, H, S> SequenceDestination<H> for &'a mut Vec<Sequence<H, S>> where S: GrammarSymbol {
    type Symbol = S;

    fn add_sequence(&mut self, seq: Sequence<H, Self::Symbol>) {
        self.push(seq);
    }
}

impl<H, S, D> SequenceDestination<H> for SequencesToProductions<H, D> where
            D: RuleContainer<History=H::Rewritten, Symbol=S>,
            H: RewriteSequence,
            H::Rewritten: Clone,
            S: GrammarSymbol {
    type Symbol = S;

    fn add_sequence(&mut self, seq: Sequence<H, Self::Symbol>) {
        self.rewrite(seq);
    }
}

impl<H, S, D> SequencesToProductions<H, D> where
            D: RuleContainer<History=H::Rewritten, Symbol=S>,
            H: RewriteSequence,
            H::Rewritten: Clone,
            S: GrammarSymbol {
    pub fn new(destination: D) -> Self {
        SequencesToProductions {
            destination: destination,
            stack: vec![],
            map: HashMap::new(),
        }
    }

    pub fn rewrite_sequences(sequence_rules: &[Sequence<H, S>], rules: D) {
        let mut rewrite = SequenceRuleBuilder::new(SequencesToProductions::new(rules));
        for rule in sequence_rules {
            rewrite = rewrite.sequence(rule.lhs)
                             .separator(rule.separator)
                             .inclusive(rule.start, rule.end)
                             .rhs_with_history(rule.rhs, &rule.history);
        }
    }

    pub fn rewrite(&mut self, top: Sequence<H, S>) {
        self.stack.clear();
        self.map.clear();
        self.stack.push(Sequence {
            lhs: top.lhs,
            rhs: top.rhs,
            start: top.start,
            end: top.end,
            separator: top.separator,
            history: top.history.sequence(&top),
        });

        while let Some(seq) = self.stack.pop() {
            assert!(seq.start <= seq.end.unwrap_or(!0));
            self.reduce(seq);
        }
    }

    fn rule(&mut self, lhs: S) -> RuleBuilder<&mut D> {
        RuleBuilder::new(&mut self.destination).rule(lhs)
    }

    fn recurse(&mut self, seq: Sequence<&H::Rewritten, S>) -> S {
        let sym_source = &mut self.destination;
        // As a placeholder
        let partial = PartialSequence {
            rhs: seq.rhs,
            separator: seq.separator,
            start: seq.start,
            end: seq.end
        };

        match self.map.entry(partial) {
            Entry::Vacant(vacant) => {
                let lhs = sym_source.sym();
                vacant.insert(lhs);
                self.stack.push(Sequence {
                    lhs: lhs,
                    rhs: seq.rhs,
                    start: seq.start,
                    end: seq.end,
                    separator: seq.separator,
                    history: seq.history.no_op(),
                });
                lhs
            }
            Entry::Occupied(lhs) => {
                *lhs.get()
            }
        }
    }

    fn reduce(&mut self, sequence: Sequence<H::Rewritten, S>) {
        let Sequence { lhs, rhs, start, end, separator, ref history } = sequence;
        let sequence = Sequence { lhs: lhs, rhs: rhs, start: start, end: end,
            separator: separator, history: history };

        match (separator, start, end) {
            (Liberal(sep), _, _) => {
                let sym1 = self.recurse(sequence.clone().separator(Proper(sep)));
                let sym2 = self.recurse(sequence.clone().separator(Trailing(sep)));
                // seq ::= sym1 | sym2
                self.rule(lhs).rhs_with_history([sym1], history.clone())
                              .rhs_with_history([sym2], history.clone());
            }
            (Trailing(sep), _, _) => {
                let sym = self.recurse(sequence.separator(Proper(sep)));
                // seq ::= sym sep
                self.rule(lhs).rhs_with_history([sym, sep], history.clone());
            }
            (_, 0, end) => {
                // seq ::= epsilon | sym
                self.rule(lhs).rhs_with_history([], history.clone());
                if end != Some(0) {
                    let sym = self.recurse(sequence.inclusive(1, end));
                    self.rule(lhs).rhs_with_history([sym], history.clone());
                }
            }
            (separator, 1, None) => {
                // seq ::= item
                self.rule(lhs).rhs_with_history([rhs], history.clone());
                // Left recursive
                // seq ::= seq sep item
                if let Separator::Proper(sep) = separator {
                    self.rule(lhs).rhs_with_history([lhs, sep, rhs], history.clone());
                } else {
                    self.rule(lhs).rhs_with_history([lhs, rhs], history.clone());
                }
            }
            (_, 1, Some(1)) => {
                self.rule(lhs).rhs_with_history([rhs], history.clone());
            }
            (_, 1, Some(2)) => {
                let sym1 = self.recurse(sequence.clone().inclusive(1, Some(1)));
                let sym2 = self.recurse(sequence.clone().inclusive(2, Some(2)));
                // seq ::= sym1 | sym2
                self.rule(lhs).rhs_with_history([sym1], history.clone())
                              .rhs_with_history([sym2], history.clone());
            }
            (separator, 1, Some(end)) => {
                let pow2 = end.next_power_of_two() / 2;
                let (seq1, seq2) = (sequence.clone().inclusive(start, Some(pow2)),
                                    sequence.clone().inclusive(start, Some(end - pow2)));
                let rhs = &[self.recurse(seq1.separator(separator.prefix_separator())),
                            self.recurse(seq2.separator(separator))];
                // seq ::= sym1 sym2
                self.rule(lhs).rhs_with_history(rhs, history.clone());
            }
            // Bug in rustc. Must use comparison.
            (Separator::Proper(sep), start, end) if start == 2 && end == Some(2) => {
                self.rule(lhs).rhs_with_history([rhs, sep, rhs], history.clone());
            }
            (separator, 2 ... 0xFFFF_FFFF, end) => {
                // to do infinity
                let (seq1, seq2) = if Some(start) == end {
                    // A "block"
                    let pow2 = start.next_power_of_two() / 2;
                    (sequence.clone().inclusive(pow2, Some(pow2)),
                     sequence.clone().inclusive(start - pow2, Some(start - pow2)))
                } else {
                    // A "span"
                    (sequence.clone().inclusive(start, Some(start)),
                     sequence.clone().inclusive(1, end.map(|n| n - start - 1)))
                };
                let rhs = &[self.recurse(seq1.separator(separator.prefix_separator())),
                            self.recurse(seq2.separator(separator))];
                // seq ::= sym1 sym2
                self.rule(lhs).rhs_with_history(rhs, history.clone());
            }
            _ => panic!()
        }
    }
}
