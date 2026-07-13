BATCH_COMPLETE is a continuation signal, not a terminal state
reasoning:deep may reduce a single monitor batch to one issue
the orchestrator MUST relaunch automatically until ALL_DONE
Never tell the operator to rerun `/autospec-run` after BATCH_COMPLETE
