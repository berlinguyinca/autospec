//! Host-set scope guard for host-local changes (issue #3683).
//!
//! A name in ssh config is not necessarily a host: it may be a
//! round-robin alias over several login nodes, each with its own
//! crontab, and `ssh name` lands on a different one every time. A
//! change to host-local state (crontab, systemd unit, `/etc` file,
//! installed binary) made through such a name has a **scope** question
//! before it has a content question: which machines did it reach? And
//! a read-back through the same alias is not verification, because the
//! read may reach a different machine than the write — with nothing in
//! the output distinguishing the two.
//!
//! The four rules this module makes checkable:
//!
//! 1. **Resolve the name first.** [`resolve`] turns `dig +short` /
//!    `getent hosts` output into a [`NameScope`]; more than one address
//!    means the name is a set of hosts, not a host.
//! 2. **Address the change to a specific host** — and record which host
//!    in the change's own note ([`AddressedChange::record`]).
//! 3. **Verify on every host in the set**, not on the one the alias
//!    returned ([`VerificationLedger`]). Only a verification named
//!    after a member of the resolved set counts.
//! 4. **Diff the hosts before assuming they are copies** ([`diff_hosts`]).
//!    "Identical" is observed, never presumed: a host with no snapshot
//!    makes the set non-identical.
//!
//! The rule in one line: `hostname` at the start of a host-modifying
//! session, and never trust a read-back that could have come from
//! somewhere else.

use std::collections::BTreeMap;
use std::fmt;

/// The scope a name resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameScope {
    /// No address was returned (NXDOMAIN, empty output). A change through
    /// this name cannot even be scoped; every check against it fails
    /// closed.
    Unresolvable,
    /// Exactly one address: the name is a host; no scope question.
    Single { address: String },
    /// More than one address: the name is a set of hosts, and any
    /// host-local change through it has a scope question.
    Multiple { addresses: Vec<String> },
}

impl NameScope {
    /// True when the name may reach more than one machine.
    pub fn is_multi(&self) -> bool {
        matches!(self, Self::Multiple { .. })
    }

    /// The set of hosts the name may reach; empty when unresolvable.
    /// Order is the resolver's output order, deduplicated.
    pub fn hosts(&self) -> Vec<String> {
        match self {
            Self::Unresolvable => Vec::new(),
            Self::Single { address } => vec![address.clone()],
            Self::Multiple { addresses } => addresses.clone(),
        }
    }

    /// True when `host` is a member of the resolved set.
    pub fn contains(&self, host: &str) -> bool {
        self.hosts().iter().any(|h| h == host)
    }
}

/// Parse `dig +short <name>` or `getent hosts <name>` output into a
/// [`NameScope`].
///
/// Each non-blank, non-comment line contributes its first
/// whitespace-separated token; tokens that are not addresses (CNAME
/// lines, prose) are ignored. Duplicate addresses collapse to the first
/// occurrence, so round-robin output with repeated records still names
/// the distinct set. Empty or all-non-address output is
/// [`NameScope::Unresolvable`].
pub fn resolve(output: &str) -> NameScope {
    let mut addresses = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let token = line.split_whitespace().next().unwrap_or_default();
        if is_address(token) && !addresses.iter().any(|a| a == token) {
            addresses.push(token.to_owned());
        }
    }
    match addresses.len() {
        0 => NameScope::Unresolvable,
        1 => NameScope::Single {
            address: addresses.into_iter().next().expect("len 1"),
        },
        _ => NameScope::Multiple { addresses },
    }
}

fn is_address(token: &str) -> bool {
    if token.contains(':') {
        // IPv6 literal; dig +short and getent hosts both emit them bare.
        return true;
    }
    let mut octets = 0;
    for part in token.split('.') {
        if part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        match part.parse::<u16>() {
            Ok(value) if value <= 255 => octets += 1,
            _ => return false,
        }
    }
    octets == 4
}

/// Why a host-local change could not be scoped or recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeError {
    /// The name resolved to no address; a change through it cannot be
    /// scoped, addressed, or verified.
    Unresolvable { via: String },
    /// The name resolves to more than one host and the change was not
    /// addressed to a specific one.
    Unscoped { via: String, addresses: Vec<String> },
    /// The recorded host is not a member of the name's resolved set —
    /// the change landed somewhere the scope question does not cover.
    NotInSet {
        via: String,
        landed_on: String,
        addresses: Vec<String>,
    },
    /// The change's note does not record the host it landed on.
    NoteMissingHost { landed_on: String },
}

impl fmt::Display for ChangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unresolvable { via } => write!(
                f,
                "{via} resolved to no address; a host-local change through it cannot be scoped or verified"
            ),
            Self::Unscoped { via, addresses } => write!(
                f,
                "{via} resolves to {} hosts ({}) but the change was not addressed to a specific one",
                addresses.len(),
                addresses.join(", ")
            ),
            Self::NotInSet {
                via,
                landed_on,
                addresses,
            } => write!(
                f,
                "change recorded as landed on {landed_on}, which is not among the hosts {via} resolves to ({})",
                addresses.join(", ")
            ),
            Self::NoteMissingHost { landed_on } => write!(
                f,
                "the change's note does not record the host it landed on ({landed_on})"
            ),
        }
    }
}

impl std::error::Error for ChangeError {}

/// A host-local change addressed to a specific host.
///
/// Construct via [`AddressedChange::record`], which fails closed when
/// the change cannot be scoped: an unresolvable name, a multi-host name
/// with no landed-on host, a landed-on host outside the resolved set, or
/// a note that does not name the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressedChange {
    /// The name the session went through (alias or host name).
    pub via: String,
    /// The specific host in the resolved set the change landed on,
    /// established from `hostname` at session start — not from the
    /// write's own output, which says nothing about which machine
    /// answered.
    pub landed_on: String,
    /// The change's own note; must record `landed_on`.
    pub note: String,
}

impl AddressedChange {
    /// Record a host-local change against the scope of the name it went
    /// through. `landed_on` must be a member of `scope`, and `note` must
    /// record it.
    pub fn record(
        via: &str,
        landed_on: &str,
        note: &str,
        scope: &NameScope,
    ) -> Result<Self, ChangeError> {
        match scope {
            NameScope::Unresolvable => {
                return Err(ChangeError::Unresolvable {
                    via: via.to_owned(),
                })
            }
            _ => {}
        }
        if landed_on.is_empty() && scope.is_multi() {
            return Err(ChangeError::Unscoped {
                via: via.to_owned(),
                addresses: scope.hosts(),
            });
        }
        if !scope.contains(landed_on) {
            return Err(ChangeError::NotInSet {
                via: via.to_owned(),
                landed_on: landed_on.to_owned(),
                addresses: scope.hosts(),
            });
        }
        if !note.contains(landed_on) {
            return Err(ChangeError::NoteMissingHost {
                landed_on: landed_on.to_owned(),
            });
        }
        Ok(Self {
            via: via.to_owned(),
            landed_on: landed_on.to_owned(),
            note: note.to_owned(),
        })
    }
}

/// Verification state for a host-local change over a resolved set.
///
/// A verification counts only when it names a member of the resolved
/// set. A read-back through the alias cannot be recorded here at all:
/// the API takes the specific host the read reached, so a read that
/// "could have come from somewhere else" has nothing to pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationLedger {
    scope: NameScope,
    verified: Vec<String>,
}

impl VerificationLedger {
    /// Ledger for `scope`. Fails closed on an unresolvable name: there
    /// is no set to verify over.
    pub fn for_scope(scope: &NameScope) -> Result<Self, ChangeError> {
        if matches!(scope, NameScope::Unresolvable) {
            return Err(ChangeError::Unresolvable {
                via: "<unknown>".to_owned(),
            });
        }
        Ok(Self {
            scope: scope.clone(),
            verified: Vec::new(),
        })
    }

    /// Record that `host` was verified directly (reached by address or
    /// host name, not through the alias). Hosts outside the resolved
    /// set are rejected: they prove nothing about the set.
    pub fn mark_verified(&mut self, host: &str) -> Result<(), ChangeError> {
        if !self.scope.contains(host) {
            return Err(ChangeError::NotInSet {
                via: "<alias>".to_owned(),
                landed_on: host.to_owned(),
                addresses: self.scope.hosts(),
            });
        }
        if !self.verified.iter().any(|h| h == host) {
            self.verified.push(host.to_owned());
        }
        Ok(())
    }

    /// Hosts in the set with no verification yet, in resolution order.
    pub fn missing(&self) -> Vec<String> {
        self.scope
            .hosts()
            .into_iter()
            .filter(|h| !self.verified.iter().any(|v| v == h))
            .collect()
    }

    /// True only when every host in the set has been verified.
    pub fn complete(&self) -> bool {
        self.missing().is_empty()
    }

    /// Hosts verified so far.
    pub fn verified(&self) -> &[String] {
        &self.verified
    }
}

/// Outcome of diffing the hosts in a set against each other.
///
/// `identical` is **observed, not presumed**: it is true only when every
/// host in the set has a snapshot and all snapshots carry exactly the
/// same lines. A single unread host makes it false. The interesting
/// output is what differs: [`HostDiff::unique_lines`] and
/// [`HostDiff::missing_lines`] name, per host, the lines that put it out
/// of step with the rest of the set.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostDiff {
    /// Hosts in the set for which no snapshot was taken.
    pub unread: Vec<String>,
    /// True only when every host in the set has a snapshot and all
    /// snapshots are line-for-line the same.
    pub identical: bool,
    /// Per host (in set order, only hosts with at least one such line):
    /// the lines that appear on that host and on no other.
    pub unique_lines: BTreeMap<String, Vec<String>>,
    /// Per host (in set order, only hosts with at least one such line):
    /// the lines that appear on other hosts but not on this one — e.g. a
    /// job removed on the host you landed on but still running on a
    /// sibling.
    pub missing_lines: BTreeMap<String, Vec<String>>,
}

/// Diff the hosts in `hosts` against each other using `snapshots`.
///
/// `hosts` is the resolved set (see [`NameScope::hosts`]); `snapshots`
/// maps each host to the observed lines. Snapshots for hosts outside the
/// set are ignored.
pub fn diff_hosts(hosts: &[String], snapshots: &BTreeMap<String, &[String]>) -> HostDiff {
    let mut diff = HostDiff::default();
    if hosts.is_empty() {
        return diff;
    }

    let present: BTreeMap<&str, &[String]> = hosts
        .iter()
        .filter_map(|h| snapshots.get(h).map(|lines| (h.as_str(), *lines)))
        .collect();
    diff.unread = hosts
        .iter()
        .filter(|h| !present.contains_key(h.as_str()))
        .cloned()
        .collect();

    // For each distinct line, which hosts carry it (in set order).
    let mut line_hosts: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for host in hosts {
        let Some(lines) = present.get(host.as_str()) else {
            continue;
        };
        for line in lines.iter() {
            let entry = line_hosts.entry(line.clone()).or_default();
            if !entry.iter().any(|h| h == host) {
                entry.push(host.clone());
            }
        }
    }

    for host in hosts {
        let Some(lines) = present.get(host.as_str()) else {
            continue;
        };
        let lines = *lines;
        let mut unique: Vec<String> = Vec::new();
        for line in lines.iter() {
            let carriers = line_hosts
                .get(line.as_str())
                .expect("line seen on this host");
            if carriers.len() == 1 {
                unique.push(line.clone());
            }
        }
        let mut missing: Vec<String> = Vec::new();
        for (line, carriers) in line_hosts.iter() {
            if !carriers.iter().any(|h| h == host) {
                missing.push(line.clone());
            }
        }
        if !unique.is_empty() {
            diff.unique_lines.insert(host.clone(), unique);
        }
        if !missing.is_empty() {
            diff.missing_lines.insert(host.clone(), missing);
        }
    }

    // "Identical" is observed: every host read, and no line exists on a
    // proper subset of the set.
    diff.identical = diff.unread.is_empty()
        && !line_hosts
            .values()
            .any(|carriers| carriers.len() != hosts.len());
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGIN1: &str = "10.0.0.1";
    const LOGIN2: &str = "10.0.0.2";

    fn two_hosts() -> Vec<String> {
        vec![LOGIN1.to_owned(), LOGIN2.to_owned()]
    }

    #[test]
    fn resolve_dig_short_round_robin_is_multiple() {
        let scope = resolve("10.0.0.2\n10.0.0.1\n");
        assert_eq!(
            scope,
            NameScope::Multiple {
                addresses: two_hosts().into_iter().rev().collect()
            }
        );
        assert!(scope.is_multi());
    }

    #[test]
    fn resolve_getent_hosts_lists_siblings() {
        let scope = resolve("10.0.0.2 hive-login2\n10.0.0.1 hive\n10.0.0.1 hive\n");
        assert_eq!(
            scope,
            NameScope::Multiple {
                addresses: vec![LOGIN2.to_owned(), LOGIN1.to_owned()]
            }
        );
    }

    #[test]
    fn resolve_single_address_is_single() {
        let scope = resolve("10.0.0.1\n");
        assert_eq!(
            scope,
            NameScope::Single {
                address: LOGIN1.to_owned()
            }
        );
        assert!(!scope.is_multi());
        assert_eq!(scope.hosts(), vec![LOGIN1.to_owned()]);
    }

    #[test]
    fn resolve_empty_or_nxdomain_is_unresolvable() {
        assert_eq!(resolve(""), NameScope::Unresolvable);
        assert_eq!(resolve("   \n\n"), NameScope::Unresolvable);
        assert_eq!(resolve("# comment only\n"), NameScope::Unresolvable);
    }

    #[test]
    fn resolve_ignores_cname_lines_and_comments() {
        let out = "hive.internal.\n# round robin\n10.0.0.1\nhive.internal.\n10.0.0.2\n";
        assert_eq!(
            resolve(out),
            NameScope::Multiple {
                addresses: two_hosts()
            }
        );
    }

    #[test]
    fn resolve_accepts_ipv6_and_rejects_prose() {
        assert_eq!(
            resolve("fd00::2\nfd00::1\n"),
            NameScope::Multiple {
                addresses: vec!["fd00::2".to_owned(), "fd00::1".to_owned()]
            }
        );
        assert_eq!(resolve("server2.internal.\n"), NameScope::Unresolvable);
        assert_eq!(resolve("999.0.0.1\n"), NameScope::Unresolvable);
    }

    #[test]
    fn record_ok_when_landed_on_is_in_set_and_note_names_it() {
        let scope = NameScope::Multiple {
            addresses: two_hosts(),
        };
        let change = AddressedChange::record(
            "hive",
            LOGIN2,
            "disabled cron job 07 on 10.0.0.2 (login2)",
            &scope,
        )
        .expect("recordable");
        assert_eq!(change.via, "hive");
        assert_eq!(change.landed_on, LOGIN2);
        assert!(change.note.contains(LOGIN2));
    }

    #[test]
    fn record_rejects_multi_host_name_without_landed_on() {
        let scope = NameScope::Multiple {
            addresses: two_hosts(),
        };
        let err = AddressedChange::record("hive", "", "disabled cron job 07", &scope)
            .expect_err("a round-robin alias needs a specific host");
        match &err {
            ChangeError::Unscoped { via, addresses } => {
                assert_eq!(via, "hive");
                assert_eq!(addresses, &two_hosts());
            }
            other => panic!("expected Unscoped, got {other:?}"),
        }
    }

    #[test]
    fn record_rejects_landed_on_outside_the_set() {
        let scope = NameScope::Multiple {
            addresses: two_hosts(),
        };
        let err = AddressedChange::record("hive", "10.9.9.9", "edit on 10.9.9.9", &scope)
            .expect_err("outside the set");
        assert!(matches!(
            err,
            ChangeError::NotInSet { ref landed_on, .. } if landed_on == "10.9.9.9"
        ));
    }

    #[test]
    fn record_rejects_single_host_mismatch() {
        let scope = NameScope::Single {
            address: LOGIN1.to_owned(),
        };
        let err = AddressedChange::record("hive", LOGIN2, "edit on 10.0.0.2", &scope)
            .expect_err("the name is one host, not this one");
        assert!(matches!(&err, ChangeError::NotInSet { .. }));
    }

    #[test]
    fn record_rejects_note_that_does_not_name_the_host() {
        let scope = NameScope::Multiple {
            addresses: two_hosts(),
        };
        let err = AddressedChange::record("hive", LOGIN2, "disabled cron job 07", &scope)
            .expect_err("the note must record the host");
        assert!(matches!(
            err,
            ChangeError::NoteMissingHost { ref landed_on } if landed_on == LOGIN2
        ));
    }

    #[test]
    fn record_rejects_unresolvable_name() {
        let scope = NameScope::Unresolvable;
        let err = AddressedChange::record("hive", LOGIN1, "edit on 10.0.0.1", &scope)
            .expect_err("cannot scope what cannot resolve");
        assert!(matches!(&err, ChangeError::Unresolvable { ref via } if via == "hive"));
    }

    #[test]
    fn ledger_is_incomplete_until_every_host_is_verified() {
        let scope = NameScope::Multiple {
            addresses: two_hosts(),
        };
        let mut ledger = VerificationLedger::for_scope(&scope).expect("scoped");
        assert!(!ledger.complete());
        assert_eq!(ledger.missing(), two_hosts());

        ledger.mark_verified(LOGIN2).expect("in set");
        assert!(
            !ledger.complete(),
            "a read-back on one node is not verification"
        );
        assert_eq!(ledger.missing(), vec![LOGIN1.to_owned()]);

        ledger.mark_verified(LOGIN1).expect("in set");
        assert!(ledger.complete());
        assert_eq!(ledger.missing(), Vec::<String>::new());
        assert_eq!(ledger.verified(), &[LOGIN2.to_owned(), LOGIN1.to_owned()]);
    }

    #[test]
    fn ledger_rejects_hosts_outside_the_set() {
        let scope = NameScope::Multiple {
            addresses: two_hosts(),
        };
        let mut ledger = VerificationLedger::for_scope(&scope).expect("scoped");
        let err = ledger
            .mark_verified("10.9.9.9")
            .expect_err("proves nothing about the set");
        assert!(matches!(
            err,
            ChangeError::NotInSet { ref landed_on, .. } if landed_on == "10.9.9.9"
        ));
        assert!(!ledger.complete());
    }

    #[test]
    fn ledger_fails_closed_on_unresolvable_scope() {
        let err = VerificationLedger::for_scope(&NameScope::Unresolvable)
            .expect_err("no set to verify over");
        assert!(matches!(&err, ChangeError::Unresolvable { .. }));
    }

    #[test]
    fn ledger_completes_single_host_scope_with_one_verification() {
        let scope = NameScope::Single {
            address: LOGIN1.to_owned(),
        };
        let mut ledger = VerificationLedger::for_scope(&scope).expect("scoped");
        assert!(!ledger.complete());
        ledger.mark_verified(LOGIN1).expect("in set");
        assert!(ledger.complete());
    }

    /// The reported incident: 67 lines on one node, 57 on the other,
    /// largely disjoint job sets, neither a superset.
    #[test]
    fn diff_hosts_names_the_disjoint_crontabs() {
        let shared = "0 */5 * * * shared-backup";
        let login1: Vec<String> = std::iter::once(shared.to_owned())
            .chain((1..=15).map(|i| format!("0 */5 * * * job1-{i}")))
            .collect();
        let login2: Vec<String> = std::iter::once(shared.to_owned())
            .chain((1..=20).map(|i| format!("0 */5 * * * job2-{i}")))
            .collect();
        let snapshots: BTreeMap<String, &[String]> = BTreeMap::from([
            (LOGIN1.to_owned(), login1.as_slice()),
            (LOGIN2.to_owned(), login2.as_slice()),
        ]);
        let diff = diff_hosts(&two_hosts(), &snapshots);

        assert!(!diff.identical, "67 vs 57 disjoint lines are not a copy");
        assert_eq!(diff.unread, Vec::<String>::new());
        assert!(
            diff.unique_lines[LOGIN1]
                .iter()
                .any(|l| l.starts_with("0 */5 * * * job1-")),
            "login1's own jobs must be named"
        );
        assert!(
            diff.unique_lines[LOGIN2]
                .iter()
                .any(|l| l.starts_with("0 */5 * * * job2-")),
            "login2's own jobs must be named"
        );
        // The job disabled on the host the alias returned is still running
        // on the sibling: it shows up as missing on the host that lacks it.
        assert!(
            diff.missing_lines[LOGIN1]
                .iter()
                .any(|l| l.starts_with("0 */5 * * * job2-")),
            "login1 is still running login2's jobs"
        );
        assert!(diff
            .unique_lines
            .values()
            .all(|v| !v.iter().any(|l| l == shared)));
    }

    #[test]
    fn diff_hosts_reports_identical_only_when_observed() {
        let lines: Vec<String> = vec!["a".to_owned(), "b".to_owned()];
        let snapshots: BTreeMap<String, &[String]> = BTreeMap::from([
            (LOGIN1.to_owned(), lines.as_slice()),
            (LOGIN2.to_owned(), lines.as_slice()),
        ]);
        let diff = diff_hosts(&two_hosts(), &snapshots);
        assert!(diff.identical);
        assert!(diff.unique_lines.is_empty());
        assert!(diff.missing_lines.is_empty());
        assert_eq!(diff.unread, Vec::<String>::new());
    }

    #[test]
    fn diff_hosts_never_presumes_identity_for_an_unread_host() {
        let lines: Vec<String> = vec!["a".to_owned()];
        let snapshots: BTreeMap<String, &[String]> =
            BTreeMap::from([(LOGIN1.to_owned(), lines.as_slice())]);
        let diff = diff_hosts(&two_hosts(), &snapshots);
        assert!(!diff.identical, "identity must be observed, not presumed");
        assert_eq!(diff.unread, vec![LOGIN2.to_owned()]);
    }

    #[test]
    fn diff_hosts_ignores_snapshots_outside_the_set_and_empty_set() {
        let lines: Vec<String> = vec!["a".to_owned()];
        let snapshots: BTreeMap<String, &[String]> =
            BTreeMap::from([(LOGIN1.to_owned(), lines.as_slice())]);
        let diff = diff_hosts(&[], &snapshots);
        assert!(!diff.identical);
        assert_eq!(diff.unread, Vec::<String>::new());

        let mut other = snapshots.clone();
        other.insert("10.9.9.9".to_owned(), lines.as_slice());
        let diff = diff_hosts(&[LOGIN1.to_owned()], &other);
        assert!(
            diff.identical,
            "one host, one snapshot, no line on a subset"
        );
        assert_eq!(diff.unread, Vec::<String>::new());
    }

    #[test]
    fn error_display_names_the_hosts() {
        let err = ChangeError::Unscoped {
            via: "hive".to_owned(),
            addresses: two_hosts(),
        };
        let text = err.to_string();
        assert!(text.contains("hive") && text.contains("10.0.0.1") && text.contains("10.0.0.2"));
        let err = ChangeError::Unresolvable {
            via: "hive".to_owned(),
        };
        assert!(err.to_string().contains("hive"));
    }
}
