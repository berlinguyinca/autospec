#!/usr/bin/env bash
# scripts/lib/model-capability-evidence.sh — §8 capability evidence levels.
#
# Sourced at $SCRIPT_DIR/lib/model-capability-evidence.sh by
# discover-model-supply.sh, before lib/model-supply-probe.sh, so _model_entry()
# can stamp each discovered model with the §7 capability shape and the §8
# evidence level backing every field of it.
#
# There are exactly FOUR levels and their routing precedence is fixed
# (docs/specs/2026-08-16-multi-model-engineering-team-design.md §8):
#
#     advertised  <  discovered  <  calibrated  <  observed
#
# advertised  declared by the model/runtime/provider — UNTRUSTED (§51)
# discovered  a probe returned the value
# calibrated  a calibration replay confirmed the capability works (§32)
# observed    real task outcomes established production performance
#
# The rule this file exists to enforce: a field is `discovered` only when a probe
# actually returned it. Everything else is `advertised` and carries the value
# "unknown" rather than a fabricated 0/false that reads like a measurement. A
# model claiming a capability therefore cannot become eligible for a gate on its
# own claim — only a calibration verdict moves a role to `calibrated`.
#
# bash 3.2+ compatible. No associative arrays, no mapfile, no RETURN traps.

# The 14 snake_case engineering roles (§3, ADR 0001 D2). Order is fixed so the
# emitted document is byte-stable across runs.
AUTOSPEC_ROLE_NAMES="orchestrator planner architect test_planner implementer
code_reviewer test_reviewer qa_verifier documentation_writer
documentation_reviewer ui_ux_reviewer security_reviewer researcher advisor"

# evidence_rank <level> — routing precedence as an integer; -1 for a level that
# is not one of the four. Callers use it to refuse a DOWNGRADE: once a role is
# `calibrated`, a later probe pass must not quietly return it to `advertised`.
evidence_rank() {
    case "$1" in
        advertised) printf '0' ;;
        discovered) printf '1' ;;
        calibrated) printf '2' ;;
        observed)   printf '3' ;;
        *)          printf -- '-1' ;;
    esac
}

# evidence_max <a> <b> — the higher-precedence of two levels. An unrecognized
# level loses to a recognized one, and two unrecognized levels collapse to
# `advertised`: an unreadable claim is the weakest possible evidence, never a
# strong one.
evidence_max() {
    _ra="$(evidence_rank "$1")"
    _rb="$(evidence_rank "$2")"
    if [ "$_ra" -lt 0 ] && [ "$_rb" -lt 0 ]; then printf 'advertised'; return 0; fi
    if [ "$_ra" -ge "$_rb" ]; then printf '%s' "$1"; else printf '%s' "$2"; fi
}

# capability_block <runtime> <ctx_tokens> <parameters> <quantization> \
#                  <weights_mb> <vision:true|false> <tool_calling:true|false>
#
# The §7-shaped additive keys for one local_models[] entry:
#   provider capability_class roles modalities automation specialties languages evidence
#
# capability_class starts at D — "experimental / unqualified" (§9). A class is
# EARNED by calibration and observation; deriving one from a parameter count is
# exactly the name-parsing this probe exists to replace.
capability_block() {
    jq -nc \
        --arg runtime "$1" \
        --arg params "$3" --arg quant "$4" \
        --arg roles "$(printf '%s' "$AUTOSPEC_ROLE_NAMES" | tr '\n' ' ')" \
        --argjson ctx_tokens "${2:-0}" \
        --argjson weights "${5:-0}" \
        --argjson vision "${6:-false}" \
        --argjson tools "${7:-false}" \
        '
        def probed(cond): if cond then "discovered" else "advertised" end;
        ($roles | split(" ") | map(select(length > 0))) as $rl |
        {
          provider: ("local-" + $runtime),
          capability_class: "D",
          roles: ($rl | map({key: ., value: false}) | from_entries),
          modalities: {text: true, vision: $vision, audio: "unknown"},
          automation: {
            filesystem: "unknown", shell: "unknown", git: "unknown",
            browser: "unknown", computer_use: "unknown",
            structured_output: "unknown", tool_calling: $tools
          },
          specialties: {},
          languages: {},
          evidence: (
            {
              context_length: probed($ctx_tokens > 0),
              parameters: probed($params != "unknown" and $params != ""),
              quantization: probed($quant != "unknown" and $quant != ""),
              weights: probed($weights > 0),
              "modalities.text": "discovered",
              "modalities.vision": "discovered",
              "modalities.audio": "advertised",
              "automation.tool_calling": "discovered",
              "automation.structured_output": "advertised",
              "automation.filesystem": "advertised",
              "automation.shell": "advertised",
              "automation.git": "advertised",
              "automation.browser": "advertised",
              "automation.computer_use": "advertised",
              capability_class: "advertised",
              specialties: "advertised",
              languages: "advertised"
            }
            + ($rl | map({key: ("roles." + .), value: "advertised"}) | from_entries)
          )
        }'
}

# apply_calibration_evidence <models_json> <fingerprint> <calibration_dir>
#
# Fold calibrate-profile.sh's per-role verdicts into the discovered model set.
# Verdicts live at <dir>/<profile>.<fingerprint>.<role>.json and are only valid
# for the hardware they were measured on, so the fingerprint is part of the name
# rather than something matched loosely.
#
# The role is taken from the FILENAME and compared inside jq with ==, never
# interpolated into a jq test() pattern: a profile or role reaching jq as a
# regex would let a dotted name match a sibling entry.
apply_calibration_evidence() {
    _models="$1"; _cal_fp="$2"; _cal_dir="$3"
    if [ ! -d "$_cal_dir" ]; then printf '%s' "$_models"; return 0; fi

    _count="$(printf '%s' "$_models" | jq 'length')"
    _idx=0
    while [ "$_idx" -lt "$_count" ]; do
        _prof="$(printf '%s' "$_models" | jq -r --argjson i "$_idx" '.[$i].profile')"
        for _vf in "$_cal_dir/$_prof.$_cal_fp."*.json; do
            if [ ! -f "$_vf" ]; then continue; fi
            _base="${_vf##*/}"; _base="${_base%.json}"; _role="${_base##*.}"
            _q="$(jq -r 'if (.qualified | type) == "boolean" then .qualified else empty end' \
                    "$_vf" 2>/dev/null || printf '')"
            case "$_q" in true|false) ;; *) continue ;; esac
            _models="$(printf '%s' "$_models" | jq -c \
                --argjson i "$_idx" --arg role "$_role" --argjson q "$_q" '
                .[$i] |= (
                  if (.roles | has($role)) then
                    .roles[$role] = $q
                    | .evidence["roles." + $role] = "calibrated"
                    | (if $q then
                         .capability_class = "C"
                         | .evidence.capability_class = "calibrated"
                       else . end)
                  else . end)')"
        done
        _idx=$((_idx + 1))
    done
    printf '%s' "$_models"
}
