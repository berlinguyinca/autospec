"""Problem detection: the checks that would have caught a node lying about itself.

This module exists because of a specific, measured failure. Both of this node's
GPUs took an `Xid 154`, llama.cpp logged `failed to initialize CUDA`, and six of
its seven models became black holes — requests hung for 45 s and then gave up.
The dashboard showed `online`, seven models, no warning, because liveness was
measured at the PROCESS level and the router process was perfectly alive.

So every test here is written against that shape of failure: a component that
answers while being unable to do its job.
"""
from nodescripts import load_script

h = load_script("health")


def texts(problems):
    return " | ".join(p["text"] for p in problems)


# --- driver faults ----------------------------------------------------------
# nvidia-smi exits ZERO here, having answered for the card it could still reach.
# The stderr is the only signal, and the collector used to discard it.
REAL_STDERR = ("Unable to determine the device handle for GPU1: "
               "0000:44:00.0: Unknown Error\n")


def test_a_card_that_stopped_answering_is_reported():
    out = h.gpu_problems(REAL_STDERR, [{"index": 0, "name": "RTX 2080 Ti"}])
    assert h.worst(out) == h.DOWN
    assert "not responding to the driver" in texts(out)


def test_a_healthy_pair_reports_nothing():
    """The refusals above only mean something if the good case is silent."""
    assert h.gpu_problems("", [{"index": 0}, {"index": 1}]) == []


def test_a_card_off_the_bus_is_reported():
    out = h.gpu_problems("GPU 0000:43:00.0 has fallen off the bus\n", [])
    assert "fallen off the bus" in texts(out)
    assert h.worst(out) == h.DOWN


def test_no_cards_at_all_is_itself_a_problem():
    assert "no GPU is visible" in texts(h.gpu_problems("", []))


def test_a_tool_that_could_not_run_is_not_reported_as_no_cards():
    """Two different facts. "Could not ask" must never render as "there are
    none" -- that is the same collapse that hid the original fault."""
    out = h.gpu_problems("", [], smi_failed=True)
    assert "could not be run" in texts(out)
    assert "no GPU is visible" not in texts(out)
    assert h.worst(out) == h.WARNING


def test_one_fault_is_reported_once_however_often_it_is_logged():
    out = h.gpu_problems(REAL_STDERR * 5, [{"index": 0}])
    assert len(out) == 1


def test_one_dead_card_is_one_fault_not_two():
    """The real line matches a specific pattern AND the vague `Unknown Error`
    one. Reporting both made a single dead card read as two faults."""
    assert len(h.gpu_problems(REAL_STDERR, [{"index": 0}])) == 1


def test_a_vague_error_on_its_own_is_still_reported():
    """The de-duplication must not silence the generic case entirely -- that
    would trade double-counting for a blind spot."""
    out = h.gpu_problems("GPU 0: Unknown Error\n", [{"index": 0}])
    assert "unknown error on a GPU" in texts(out)


# --- runtime faults ---------------------------------------------------------

def test_the_exact_journal_line_from_the_real_outage_is_caught():
    out = h.runtime_problems(True, "ggml_cuda_init: failed to initialize CUDA: "
                                   "unknown error", True)
    assert h.worst(out) == h.DOWN
    assert "could not initialise CUDA" in texts(out)


def test_an_xid_says_a_reboot_is_probably_needed():
    out = h.runtime_problems(True, "NVRM: Xid (PCI:0000:43:00): 154, GPU "
                                   "recovery action changed", True)
    assert "reboot" in texts(out)
    assert h.worst(out) == h.DOWN


def test_a_runtime_that_answers_nothing_is_down():
    assert h.worst(h.runtime_problems(False, "", True)) == h.DOWN


def test_a_healthy_runtime_reports_nothing():
    assert h.runtime_problems(True, "srv  loading model\nall good", True) == []


def test_an_unreadable_log_is_reported_rather_than_assumed_clean():
    """Silence that looks like health is the failure mode this guards."""
    out = h.runtime_problems(True, "", False)
    assert "log could not be read" in texts(out)
    assert h.worst(out) == h.WARNING


def test_an_unreadable_log_does_not_also_claim_no_faults():
    # The fault patterns must not be evaluated against text nobody could read.
    out = h.runtime_problems(True, "Xid (PCI:0000:43:00): 154", False)
    assert len(out) == 1 and "could not be read" in texts(out)


# --- fleet members ----------------------------------------------------------

def test_a_server_that_is_not_answering_is_down():
    out = h.server_problems({"id": "bender", "state": "offline"})
    assert h.worst(out) == h.DOWN
    assert out[0]["where"] == "bender"


def test_an_online_server_serving_nothing_is_down():
    """Today's failure seen from the other side: attached, and able to serve
    nothing. It used to render as a healthy server."""
    out = h.server_problems({"id": "box", "state": "online", "models": []})
    assert h.worst(out) == h.DOWN
    assert "reporting no models" in texts(out)


def test_a_healthy_server_reports_nothing():
    assert h.server_problems({"id": "box", "state": "online",
                              "models": ["m"], "kind": "tunnel",
                              "idle_pipes": 4}) == []


def test_a_probe_failure_never_quotes_the_address_it_failed_to_reach():
    """This list is public, and an upstream error string names a host and port."""
    out = h.server_problems({"id": "box", "state": "online", "models": ["m"],
                             "error": "connect to gpu-box.invalid:8080 refused"})
    assert "gpu-box.invalid" not in repr(out)
    assert "last probe of it failed" in texts(out)


def test_a_registered_server_not_yet_seen_is_a_warning_not_a_failure():
    out = h.server_problems({"id": "box", "state": "unknown"})
    assert h.worst(out) == h.WARNING


def test_a_tunnel_with_no_spare_connection_is_flagged():
    out = h.server_problems({"id": "box", "state": "online", "models": ["m"],
                             "kind": "tunnel", "idle_pipes": 0})
    assert "no spare connection" in texts(out)


def test_a_direct_server_is_not_judged_on_pipes_it_cannot_have():
    assert h.server_problems({"id": "box", "state": "online", "models": ["m"],
                              "kind": "static", "idle_pipes": None}) == []


def test_configuration_faults_are_folded_into_the_same_list():
    out = h.server_problems({"id": "box", "state": "online", "models": ["m"],
                             "problems": ["no usable base_url"]})
    assert "configuration: no usable base_url" in texts(out)


# --- what a caller actually feels -------------------------------------------

def test_models_whose_only_server_is_broken_are_reported():
    """THE check this module was written for. Six models stayed advertised by a
    node whose runtime was dead, and every request for one hung."""
    servers = [
        {"id": "local", "state": "online", "models": ["a", "b", "shared"]},
        {"id": "bender", "state": "online", "models": ["shared"], "kind": "tunnel",
         "idle_pipes": 4},
    ]
    # local goes down: 'a' and 'b' lose their only host, 'shared' survives.
    servers[0] = {"id": "local", "state": "online", "models": []}
    out = h.orphaned_models(["a", "b", "shared"], servers)
    assert h.worst(out) == h.DOWN
    assert "2 advertised model(s) have no healthy server" in texts(out)
    assert "a, b" in texts(out)


def test_nothing_is_reported_when_every_model_has_a_healthy_host():
    servers = [{"id": "local", "state": "online", "models": ["a", "b"]}]
    assert h.orphaned_models(["a", "b"], servers) == []


def test_a_model_on_a_warned_but_working_server_is_not_orphaned():
    """A warning is not a loss of capability. Treating it as one would take a
    working server out of the picture on the page."""
    servers = [{"id": "box", "state": "online", "models": ["a"], "kind": "tunnel",
                "idle_pipes": 0}]
    assert h.orphaned_models(["a"], servers) == []


def test_a_long_orphan_list_is_summarised_with_its_true_count():
    servers = [{"id": "local", "state": "offline", "models": []}]
    out = h.orphaned_models([f"m{i}" for i in range(9)], servers)
    assert "9 advertised model(s)" in texts(out)
    assert texts(out).endswith("…")


# --- severity ---------------------------------------------------------------

def test_the_worst_severity_wins():
    assert h.worst([{"severity": h.WARNING}, {"severity": h.DOWN}]) == h.DOWN
    assert h.worst([{"severity": h.WARNING}, {"severity": h.DEGRADED}]) == h.DEGRADED
    assert h.worst([]) is None and h.worst(None) is None


def test_a_nodes_own_faults_count_against_its_models():
    """The link that was missing live. llama.cpp's /health answered 200 while
    CUDA was dead, so the node sat in the fleet as `online` with seven models it
    could not serve -- and nothing marked them lost.

    Those faults come from a journal, not from anything visible in the row, so
    orphaned_models must honour a `faults` list the caller already computed rather
    than re-deriving one.
    """
    servers = [{"id": "local", "state": "online", "models": ["a", "b"],
                "faults": [{"severity": h.DOWN,
                            "text": "the runtime could not initialise CUDA",
                            "where": "this node"}]}]
    out = h.orphaned_models(["a", "b"], servers)
    assert h.worst(out) == h.DOWN
    assert "2 advertised model(s) have no healthy server" in texts(out)


def test_precomputed_faults_are_honoured_over_re_derivation():
    """An empty precomputed list means "asked, and nothing was wrong" -- it must
    not be treated as "not asked" and quietly recomputed."""
    servers = [{"id": "local", "state": "online", "models": ["a"], "faults": []}]
    assert h.orphaned_models(["a"], servers) == []


# --- false positives are worse than gaps ------------------------------------

def test_the_routers_own_eviction_line_is_not_a_memory_failure():
    """The exact line that fooled the first version of this module. `OOM`
    case-insensitively matches "to make rOOM for" -- a healthy LRU eviction --
    so a node with no memory trouble was reported as having run out. It was
    reported to a person, who then asked for a memory fix that was not needed.
    """
    line = ("srv  ensure_model: evicting idle LRU name=qwen3.8-27b to make "
            "room for name=qwen3.8-27b-uncensored")
    assert h.runtime_problems(True, line, True) == []


def test_a_real_memory_failure_is_still_caught():
    """The narrowing must not trade a false positive for a blind spot."""
    # The kernel capitalises the phrase, so the phrase must be case-insensitive
    # even though the acronym cannot be.
    for line in ("ggml_backend_cuda_buffer_type_alloc_buffer: failed to allocate",
                 "CUDA error: out of memory",
                 "kernel: Out of memory: Killed process 1234",
                 "oom-kill:constraint=CONSTRAINT_NONE",
                 "llama_new_context_with_model: OOM"):
        assert h.runtime_problems(True, line, True), line


def test_the_word_room_never_triggers_it():
    for line in ("making room", "no room left in the cache", "ROOM", "bedroom"):
        assert h.runtime_problems(True, line, True) == [], line
