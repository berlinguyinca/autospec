pub mod commands;

commands! {
    validate => "Run configured validation gates", diagnostic;
    repair_loop as "repair-loop" => "Observe repair loops", direct;
}
