# Spec: cyclic fixture (negative case)

A proposed decomposition whose hard-dependency edges form a cycle
(#9001 -> #9002 -> #9003 -> #9004 -> #9001). The DAG analyzer must reject
this graph (AS-DAG-010, spec §34 AC-6) before any issue is filed.
