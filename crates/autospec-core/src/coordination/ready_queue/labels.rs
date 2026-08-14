pub(super) const SERIAL_LABELS: &[&str] = &[
    "reasoning:deep",
    "priority:high",
    "regression",
    "audit",
    "release",
];

pub(super) const BLOCKING_LABELS: &[(&str, &str)] = &[
    ("needs-classify", "needs_classify"),
    ("groom:proposed", "groom_proposed"),
    ("autospec:needs-human", "autospec_needs_human"),
    (
        "autospec:blocked-prerequisite",
        "security_prerequisite_blocked",
    ),
];
