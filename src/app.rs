//! Router assembly.


use axum::extract::DefaultBodyLimit;
use axum::middleware::from_fn_with_state;
use axum::routing::{delete, get, post, put};
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use crate::auth::require_auth;
use crate::handlers;
use crate::state::AppState;

/// Build the complete application router.
pub fn build(state: AppState) -> Router {
    // Public routes: bootstrap check, login, first-run setup.
    // The WebSocket terminal is public at the router level because browsers
    // cannot set Authorization headers on a WS handshake; it validates the
    // session token itself via a ?token= query parameter.
    let public = Router::new()
        .route("/api/users/bootstrap", get(handlers::users::bootstrap_status))
        .route("/api/users/bootstrap", post(handlers::users::bootstrap))
        .route("/api/auth/login", post(handlers::auth::login))
        .route("/api/term/ws", get(crate::terminal::ws_handler))
        // VNC WebSocket proxy — public for the same reason as term/ws (browsers
        // can't set Authorization on WS handshake). Token via ?token= query.
        .route("/api/bhyve/vms/{name}/vnc", get(crate::terminal::vnc_ws_handler))
        // Unified SSE stream for background tasks (pkg install/delete,
        // bhyve init, etc.). EventSource cannot set Authorization headers,
        // so token is validated via query param inside the handler.
        .route("/api/tasks/{id}/stream", get(crate::bgtask::stream_handler));

    // Authenticated routes.
    let api = Router::new()
        .route("/api/auth/logout", post(handlers::auth::logout))
        .route("/api/auth/me", get(handlers::auth::me))
        .route("/api/system/info", get(handlers::system::system_info))
        .route("/api/system/metrics", get(handlers::system::system_metrics))
        .route("/api/system/shutdown", post(handlers::system::shutdown))
        .route("/api/system/reboot", post(handlers::system::reboot))
        .route("/api/users", get(handlers::users::list_users))
        .route("/api/users", post(handlers::users::create_user))
        .route("/api/users/{id}", put(handlers::users::update_user))
        .route("/api/users/{id}", delete(handlers::users::delete_user))
        .route("/api/audit", get(handlers::audit::list))
        // --- System accounts (FreeBSD users & groups) ---
        .route("/api/accounts/users", get(handlers::accounts::list_users).post(handlers::accounts::create_user))
        .route("/api/accounts/users/{name}", put(handlers::accounts::update_user).delete(handlers::accounts::delete_user))
        .route("/api/accounts/groups", get(handlers::accounts::list_groups).post(handlers::accounts::create_group))
        .route("/api/accounts/groups/{name}", put(handlers::accounts::update_group).delete(handlers::accounts::delete_group))
        .route("/api/accounts/shells", get(handlers::accounts::list_shells))
        // --- sysctl ---
        .route("/api/sysctl", get(handlers::sysctl::list))
        .route("/api/sysctl/{name}", put(handlers::sysctl::set).delete(handlers::sysctl::reset))
        // --- rc.conf (sysrc) ---
        .route("/api/rcconf", get(handlers::rcconf::list).put(handlers::rcconf::set))
        .route("/api/rcconf", delete(handlers::rcconf::delete))
        // --- crontab ---
        .route("/api/crontab", get(handlers::crontab::list).post(handlers::crontab::create).put(handlers::crontab::update))
        .route("/api/crontab", delete(handlers::crontab::delete))
        .route("/api/crontab/targets", get(handlers::crontab::targets))
        .route("/api/network/interfaces", get(handlers::network::list_interfaces).post(handlers::network::interface_create))
        .route("/api/network/interfaces/{name}", get(handlers::network::interface_detail).put(handlers::network::interface_update).delete(handlers::network::interface_destroy))
        .route("/api/network/interfaces/{name}/apply", post(handlers::network::interface_apply))
        .route("/api/network/routes", get(handlers::network::list_routes))
        .route("/api/network/gateway", get(handlers::network::default_gateway).put(handlers::network::set_default_gateway))
        .route("/api/network/static-routes", get(handlers::network::list_static_routes).post(handlers::network::create_static_route))
        .route("/api/network/static-routes/{name}", put(handlers::network::update_static_route).delete(handlers::network::delete_static_route))
        .route("/api/network/dns", get(handlers::network::dns_config))
        .route("/api/network/dns/nameservers", put(handlers::network::set_nameservers))
        .route("/api/services", get(handlers::services::list))
        .route("/api/services/{name}/{action}", post(handlers::services::control))
        // --- Firewall (ipfw / pf dual-driver) ---
        .route("/api/firewall/status", get(handlers::firewall::status))
        .route("/api/firewall/initialize", post(handlers::firewall::initialize))
        .route("/api/firewall/switch", post(handlers::firewall::switch))
        .route("/api/firewall/enable", post(handlers::firewall::enable))
        .route("/api/firewall/disable", post(handlers::firewall::disable))
        .route("/api/firewall/mode", put(handlers::firewall::set_mode))
        .route("/api/firewall/rules", get(handlers::firewall::list_rules).post(handlers::firewall::create_rule))
        .route("/api/firewall/rules/{id}", put(handlers::firewall::update_rule).delete(handlers::firewall::delete_rule))
        .route("/api/firewall/rules/{id}/toggle", put(handlers::firewall::toggle_rule))
        .route("/api/firewall/rules/reorder", put(handlers::firewall::reorder_rules))
        .route("/api/firewall/apply", post(handlers::firewall::apply))
        .route("/api/firewall/confirm", post(handlers::firewall::confirm))
        .route("/api/firewall/rollback", post(handlers::firewall::rollback))
        .route("/api/firewall/discard", post(handlers::firewall::discard))
        .route("/api/firewall/config", get(handlers::firewall::config))
        .route("/api/firewall/tables", get(handlers::firewall::list_tables).post(handlers::firewall::create_table))
        .route("/api/firewall/tables/{id}", put(handlers::firewall::update_table).delete(handlers::firewall::delete_table))
        .route("/api/firewall/tables/{id}/entries", post(handlers::firewall::add_entry))
        .route("/api/firewall/tables/{id}/entries/{eid}", delete(handlers::firewall::delete_entry))
        .route("/api/firewall/nat/rules", get(handlers::firewall::list_nat_rules).post(handlers::firewall::create_nat_rule))
        .route("/api/firewall/nat/rules/{id}", put(handlers::firewall::update_nat_rule).delete(handlers::firewall::delete_nat_rule))
        .route("/api/firewall/nat/rules/{id}/toggle", put(handlers::firewall::toggle_nat_rule))
        .route("/api/firewall/nat/rules/reorder", put(handlers::firewall::reorder_nat_rules))
        .route("/api/jails", get(handlers::jails::list))
        .route("/api/jails/init", get(handlers::jails::init_status).post(handlers::jails::jail_init))
        .route("/api/jails/create", post(handlers::jails::jail_create))
        .route("/api/jails/config/global", get(handlers::jails::get_global_conf).put(handlers::jails::put_global_conf))
        .route("/api/jails/config/devfs", get(handlers::jails::get_devfs_rules).put(handlers::jails::put_devfs_rules))
        .route("/api/jails/config/resolv", get(handlers::jails::get_resolv_conf).put(handlers::jails::put_resolv_conf))
        .route("/api/jails/{name}", get(handlers::jails::detail).delete(handlers::jails::jail_delete).put(handlers::jails::jail_update))
        .route("/api/jails/{name}/start", post(handlers::jails::jail_start))
        .route("/api/jails/{name}/stop", post(handlers::jails::jail_stop))
        .route("/api/jails/{name}/fstab", get(handlers::jails::fstab_list).put(handlers::jails::fstab_update))
        .route("/api/jails/bases", get(handlers::jails::base_list).post(handlers::jails::base_import))
        .route("/api/jails/bases/mirrors", get(handlers::jails::mirror_list))
        .route("/api/jails/bases/snapshots", get(handlers::jails::zfs_snapshot_list))
        .route("/api/jails/bases/{name}", delete(handlers::jails::base_destroy).put(handlers::jails::base_update))
        .route("/api/bhyve/vms", get(handlers::bhyve::list_vms).post(handlers::bhyve::create_vm))
        .route("/api/bhyve/status", get(handlers::bhyve::status))
        .route("/api/bhyve/init", post(handlers::bhyve::init))
        .route("/api/bhyve/vms/{name}", get(handlers::bhyve::vm_detail).put(handlers::bhyve::update_vm_config).delete(handlers::bhyve::destroy_vm))
        .route("/api/bhyve/vms/{name}/state", get(handlers::bhyve::vm_state))
        .route("/api/bhyve/vms/{name}/disk-resources", get(handlers::bhyve::disk_resources))
        .route("/api/bhyve/vms/{name}/disks", post(handlers::bhyve::add_disk))
        .route("/api/bhyve/vms/{name}/disks/{index}", delete(handlers::bhyve::delete_disk))
        .route("/api/bhyve/vms/{name}/networks/{index}", delete(handlers::bhyve::delete_network))
        .route("/api/bhyve/vms/{name}/start", post(handlers::bhyve::vm_start))
        .route("/api/bhyve/vms/{name}/stop", post(handlers::bhyve::vm_stop))
        .route("/api/bhyve/vms/{name}/poweroff", post(handlers::bhyve::vm_poweroff))
        .route("/api/bhyve/vms/{name}/install", post(handlers::bhyve::vm_install))
        .route("/api/bhyve/images", get(handlers::bhyve::list_images).post(handlers::bhyve::create_image))
        .route("/api/bhyve/images/{uuid}/provision", post(handlers::bhyve::provision_image))
        .route("/api/bhyve/images/{uuid}", delete(handlers::bhyve::destroy_image))
        .route("/api/bhyve/switches", get(handlers::bhyve::list_switches).post(handlers::bhyve::create_switch))
        .route("/api/bhyve/switches/{name}", get(handlers::bhyve::switch_detail).delete(handlers::bhyve::delete_switch))
        .route("/api/bhyve/switches/{name}/vlan", put(handlers::bhyve::switch_vlan))
        .route("/api/bhyve/switches/{name}/address", put(handlers::bhyve::switch_address))
        .route("/api/bhyve/switches/{name}/private", put(handlers::bhyve::switch_private))
        .route("/api/bhyve/switches/{name}/ports", post(handlers::bhyve::switch_add_port))
        .route("/api/bhyve/switches/{name}/ports/{interface}", delete(handlers::bhyve::switch_remove_port))
        .route("/api/bhyve/datastores", get(handlers::bhyve::list_datastores).post(handlers::bhyve::create_datastore))
        .route("/api/bhyve/datastores/{name}", delete(handlers::bhyve::delete_datastore))
        .route("/api/bhyve/templates", get(handlers::bhyve::list_templates))
        .route("/api/bhyve/isos", get(handlers::bhyve::list_isos).post(handlers::bhyve::fetch_iso))
        .route("/api/bhyve/isos/{name}", delete(handlers::bhyve::delete_iso))
        .route("/api/bhyve/img-files", get(handlers::bhyve::list_img_files))
        // --- ZFS ---
        .route("/api/zfs/pools", get(handlers::zfs::pool_list).post(handlers::zfs::pool_create))
        .route("/api/zfs/pools/available-disks", get(handlers::zfs::available_disks))
        .route("/api/zfs/pools/importable", get(handlers::zfs::pool_importable))
        .route("/api/zfs/pools/import", post(handlers::zfs::pool_import))
        .route("/api/zfs/pools/{name}", get(handlers::zfs::pool_status).delete(handlers::zfs::pool_destroy))
        .route("/api/zfs/pools/{name}/add", post(handlers::zfs::pool_add_vdev))
        .route("/api/zfs/pools/{name}/attach", post(handlers::zfs::pool_attach))
        .route("/api/zfs/pools/{name}/detach", post(handlers::zfs::pool_detach))
        .route("/api/zfs/pools/{name}/replace", post(handlers::zfs::pool_replace))
        .route("/api/zfs/pools/{name}/export", post(handlers::zfs::pool_export))
        .route("/api/zfs/pools/{name}/scrub", post(handlers::zfs::pool_scrub))
        .route("/api/zfs/pools/{name}/scrub/stop", post(handlers::zfs::pool_scrub_stop))
        .route("/api/zfs/datasets", get(handlers::zfs::dataset_list).post(handlers::zfs::dataset_create))
        .route("/api/zfs/dataset/destroy", delete(handlers::zfs::dataset_destroy))
        .route("/api/zfs/dataset/properties", get(handlers::zfs::dataset_properties).put(handlers::zfs::dataset_set))
        .route("/api/zfs/dataset/inherit", post(handlers::zfs::dataset_inherit))
        .route("/api/zfs/dataset/prop-schema", get(handlers::zfs::dataset_prop_schema))
        .route("/api/zfs/snapshots", get(handlers::zfs::snapshot_list).post(handlers::zfs::snapshot_create))
        .route("/api/zfs/snapshot/destroy", delete(handlers::zfs::snapshot_destroy))
        .route("/api/zfs/snapshot/rollback", post(handlers::zfs::snapshot_rollback))
        .route("/api/zfs/snapshot/clone", post(handlers::zfs::snapshot_clone))
        .route("/api/filesystem/overview", get(handlers::filesystem::overview))
        .route("/api/filesystem/disks", get(handlers::filesystem::disk_detail))
        .route("/api/filesystem/disks/{name}/smart", get(handlers::filesystem::disk_smart))
        // --- File manager ---
        .route("/api/files/list", get(handlers::files::list))
        .route("/api/files/stat", get(handlers::files::stat))
        .route("/api/files/mkdir", post(handlers::files::mkdir))
        .route("/api/files/rename", post(handlers::files::rename))
        .route("/api/files", delete(handlers::files::delete))
        .route("/api/files/download", get(handlers::files::download))
        .route("/api/files/accounts", get(handlers::files::accounts))
        .route("/api/files/chmod", put(handlers::files::chmod))
        .route("/api/files/chown", put(handlers::files::chown))
        .route("/api/monitor/series", get(crate::monitor::series))
        .route("/api/monitor/grouped", get(crate::monitor::grouped))
        .route("/api/monitor/aggregate", get(crate::monitor::aggregate))
        // --- System mail (mbox) ---
        .route("/api/mail/boxes", get(handlers::mail::list_mailboxes))
        .route("/api/mail/{user}", get(handlers::mail::list_mails).delete(handlers::mail::clear_mailbox))
        .route("/api/mail/{user}/delete", post(handlers::mail::batch_delete))
        .route("/api/mail/{user}/{index}", get(handlers::mail::read_mail).delete(handlers::mail::delete_mail))
        .route("/api/mail/{user}/{index}/read", put(handlers::mail::mark_read))
        .route("/api/mail/{user}/{index}/unread", put(handlers::mail::mark_unread))
        // --- Debug / diagnostics ---
        .route("/api/debug/jemalloc-stats", get(handlers::debug::jemalloc_stats))
        .route("/api/debug/tokio-metrics", get(handlers::debug::tokio_metrics))
        // --- Scheduler status ---
        .route("/api/scheduler/status", get(crate::scheduler::status))
        // --- pkg (package management) ---
        .route("/api/pkg/packages", get(handlers::pkg::list_packages))
        .route("/api/pkg/search", get(handlers::pkg::search))
        .route("/api/pkg/preview", post(handlers::pkg::preview))
        .route("/api/pkg/install", post(handlers::pkg::install))
        .route("/api/pkg/delete", post(handlers::pkg::delete))
        .route("/api/pkg/upgrade", post(handlers::pkg::upgrade))
        .route("/api/pkg/autoremove", post(handlers::pkg::autoremove))
        .route("/api/pkg/lock", post(handlers::pkg::lock))
        .route("/api/pkg/unlock", post(handlers::pkg::unlock))
        .route("/api/pkg/tasks/{id}", get(handlers::pkg::task_status))
        .route("/api/tasks/{id}", get(handlers::pkg::task_status))
        .route("/api/pkg/packages/{name}", get(handlers::pkg::package_detail))
        .route("/api/pkg/packages/{name}/files", get(handlers::pkg::package_files))
        // --- pkg repos (repository management) ---
        .route("/api/pkg/repos", get(handlers::pkg::list_repos).post(handlers::pkg::create_repo))
        .route("/api/pkg/repos/apply_mirror", post(handlers::pkg::apply_mirror))
        .route("/api/pkg/repos/{name}", put(handlers::pkg::update_repo).delete(handlers::pkg::delete_repo))
        .route("/api/pkg/repos/update", post(handlers::pkg::repo_update))
        // --- SMB (Samba file sharing) ---
        .route("/api/smb/status", get(handlers::smb::status))
        .route("/api/smb/init", post(handlers::smb::init))
        .route("/api/smb/config", get(handlers::smb::get_config).put(handlers::smb::update_config))
        .route("/api/smb/shares", get(handlers::smb::list_shares).post(handlers::smb::create_share))
        .route("/api/smb/shares/{name}", put(handlers::smb::update_share).delete(handlers::smb::delete_share))
        .route("/api/smb/users", get(handlers::smb::list_users).post(handlers::smb::create_user))
        .route("/api/smb/sysusers", get(handlers::smb::list_sysusers))
        .route("/api/smb/users/{name}", delete(handlers::smb::delete_user))
        .route("/api/smb/users/{name}/password", put(handlers::smb::change_password))
        .route("/api/smb/service/{action}", post(handlers::smb::service_control))
        // --- Rsync sync tasks ---
        .route("/api/rsync/status", get(handlers::rsync::status))
        .route("/api/rsync/init", post(handlers::rsync::init))
        .route("/api/rsync/tasks", get(handlers::rsync::list_tasks).post(handlers::rsync::create_task))
        .route("/api/rsync/tasks/{id}", put(handlers::rsync::update_task).delete(handlers::rsync::delete_task))
        .route("/api/rsync/tasks/{id}/run", post(handlers::rsync::run_task))
        .route("/api/rsync/browse", get(handlers::rsync::browse))
        .layer(from_fn_with_state(state.clone(), require_auth));

    // File upload sends raw bytes as the request body and can be large;
    // disable axum's default 2 MiB body limit for this route only.
    let upload_api = Router::new()
        .route("/api/files/upload", post(handlers::files::upload))
        .layer(DefaultBodyLimit::disable())
        .layer(from_fn_with_state(state.clone(), require_auth));

    Router::new()
        .merge(public)
        .merge(api)
        .merge(upload_api)
        .fallback(crate::web_assets::serve_asset)
        .layer(CorsLayer::very_permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
