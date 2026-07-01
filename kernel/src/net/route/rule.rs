// SPDX-License-Identifier: MPL-2.0

use super::RouteTableId;
use crate::prelude::*;

/// One routing policy rule.
///
/// Rules are evaluated by ascending priority and decide which route table
/// participates in lookup.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct Rule {
    priority: u32,
    action: RuleAction,
    table: Option<RouteTableId>,
}

impl Rule {
    pub(super) const fn lookup(priority: u32, table: RouteTableId) -> Self {
        Self {
            priority,
            action: RuleAction::Lookup,
            table: Some(table),
        }
    }

    pub(super) fn action(&self) -> RuleAction {
        self.action
    }

    pub(super) fn table(&self) -> Option<RouteTableId> {
        self.table
    }
}

/// Rule action.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum RuleAction {
    Lookup,
    #[expect(dead_code)]
    Unreachable,
    #[expect(dead_code)]
    Prohibit,
    #[expect(dead_code)]
    Blackhole,
}

/// The ordered routing policy database.
///
/// Linux evaluates routing policy rules in priority order, so the list is a
/// `BTreeSet` keyed by the derived ordering of `Rule`. The default rules
/// mirror Linux's local, main, and default table lookup order.
#[derive(Clone, Debug)]
pub(super) struct RuleList {
    rules: BTreeSet<Rule>,
}

impl RuleList {
    pub(super) fn iter(&self) -> impl Iterator<Item = &Rule> {
        self.rules.iter()
    }
}

impl Default for RuleList {
    fn default() -> Self {
        let rules = BTreeSet::from([
            Rule::lookup(0, RouteTableId::LOCAL),
            Rule::lookup(32766, RouteTableId::MAIN),
            Rule::lookup(32767, RouteTableId::DEFAULT),
        ]);
        Self { rules }
    }
}
