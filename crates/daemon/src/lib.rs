pub mod automation_sessions;
pub mod current_chat;
pub mod cutover;
pub mod embedded {
    refinery::embed_migrations!("migrations");
}
pub mod model_runtime;
pub mod permission_probe;
pub mod ui_relaunch;
pub mod unified_work;
pub mod work_coordinator;
