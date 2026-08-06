// SPDX-License-Identifier: MPL-2.0

use super::{
    RouteAddressFamily, RouteEntry, RouteLookupKey, RouteTableId,
    rule::{Rule, RuleAction, RulePriority},
    table::RouteTable,
};
use crate::prelude::*;

/// The manager that owns the routing policy rules and tables for one IP address family.
pub(super) struct RouteManager<A: RouteAddressFamily> {
    rules: BTreeMap<RulePriority, Vec<Rule>>,
    tables: BTreeMap<RouteTableId, RouteTable<A>>,
}

impl<A: RouteAddressFamily> RouteManager<A> {
    pub(super) fn new(default_rules: &[Rule], routes: Vec<RouteEntry<A>>) -> Self {
        let mut rules = BTreeMap::<RulePriority, Vec<Rule>>::new();
        let mut tables = BTreeMap::new();

        for rule in default_rules {
            if let RuleAction::Lookup(table_id) = rule.action() {
                tables.entry(*table_id).or_insert_with(RouteTable::new);
            }
            rules.entry(rule.priority()).or_default().push(rule.clone());
        }

        for route in routes {
            let table_id = route.table_id();
            tables.get_mut(&table_id).unwrap().insert(route);
        }

        Self { rules, tables }
    }

    pub(super) fn lookup_entry(&self, key: &RouteLookupKey<A>) -> Result<RouteEntry<A>> {
        for rules in self.rules.values() {
            for rule in rules {
                match rule.action() {
                    RuleAction::Lookup(table_id) => {
                        let Some(table) = self.tables.get(table_id) else {
                            continue;
                        };
                        if let Some(route) = table.lookup(key) {
                            return Ok(route);
                        }
                    }
                    RuleAction::Unreachable => {
                        return_errno_with_message!(
                            Errno::ENETUNREACH,
                            "the route rule rejects lookup"
                        );
                    }
                    RuleAction::Prohibit => {
                        return_errno_with_message!(
                            Errno::EACCES,
                            "the route rule prohibits lookup"
                        );
                    }
                    RuleAction::Blackhole => {
                        return_errno_with_message!(Errno::EINVAL, "the route rule discards lookup");
                    }
                }
            }
        }

        return_errno_with_message!(Errno::ENETUNREACH, "no route to the destination")
    }

    pub(super) fn lookup_in_local_table(&self, key: &RouteLookupKey<A>) -> Option<RouteEntry<A>> {
        self.tables.get(&RouteTableId::LOCAL)?.lookup(key)
    }
}
