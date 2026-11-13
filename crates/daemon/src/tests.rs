use super::*;
use crate::compression::*;
use crate::hash::*;
use crate::logs::*;
use crate::scan::*;
use crate::status::*;
use crate::thumbnails::*;
use std::io::Write;
use std::time::SystemTime;
use vrc_photon_ai_host::test_support::sample_manifest;
use vrc_photon_ai_protocol::Probe;
use vrc_photon_core::ai::PluginInstallation;
use vrc_photon_core::model::{
    CompressionOrigin, CompressionProfile, DeletionStatus, JobStatusCounts, NewPhoto, PhotoContext,
    SendOrigin, SendStatus,
};
use vrc_photon_core::settings::{DiscordAutoMode, save_discord_webhook};

#[test]
fn localized_error_builds_text_and_json_args() {
    let error = LocalizedError::new("daemon.scan_failed").arg("count", 3);
    assert!(error.message_in("ja").contains('3'));
    assert!(error.message_in("en").contains('3'));
    assert_eq!(error.args_json(), r#"{"count":"3"}"#);
}

#[test]
fn revision_bump_retries_after_a_failure() {
    let mut pending = true;
    let mut calls = 0;
    let first = retry_revision(&mut pending, || {
        calls += 1;
        Err(anyhow::anyhow!("database busy"))
    });
    assert!(!first);
    assert!(pending);
    let second = retry_revision(&mut pending, || {
        calls += 1;
        Ok(())
    });
    assert!(second);
    assert!(!pending);
    assert_eq!(calls, 2);
}

#[test]
fn reads_only_complete_lines_with_offsets() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("output_log_test.txt");
    let mut file = File::create(&path).unwrap();
    write!(file, "line1\nline2\npart").unwrap();
    drop(file);

    let (lines, offset) = read_new_lines(&path, 0).unwrap();
    assert_eq!(
        lines,
        vec![("line1".to_string(), 6), ("line2".to_string(), 12)]
    );
    assert_eq!(offset, 12);
}

#[test]
fn read_new_lines_stops_at_the_line_budget() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("output_log_test.txt");
    std::fs::write(&path, "one\ntwo\nthree\nfour").unwrap();

    let (lines, offset) = read_new_lines_bounded(&path, 0, 2, 1024).unwrap();
    assert_eq!(lines, vec![("one".to_string(), 4), ("two".to_string(), 8)]);
    assert_eq!(offset, 8);

    let (lines, offset) = read_new_lines_bounded(&path, offset, 2, 1024).unwrap();
    assert_eq!(lines, vec![("three".to_string(), 14)]);
    assert_eq!(offset, 14, "the partial last line is left for next time");
}

#[test]
fn read_new_lines_stops_at_the_byte_budget_without_consuming_a_partial_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("output_log_test.txt");
    std::fs::write(&path, "line1\nline2\n").unwrap();

    let (lines, offset) = read_new_lines_bounded(&path, 0, 100, 8).unwrap();
    assert_eq!(lines, vec![("line1".to_string(), 6)]);
    assert_eq!(offset, 6);

    let (lines, offset) = read_new_lines_bounded(&path, offset, 100, 8).unwrap();
    assert_eq!(lines, vec![("line2".to_string(), 12)]);
    assert_eq!(offset, 12);
}

#[test]
fn poison_line_is_skipped_only_after_three_attempts() {
    let mut failed_offset = None;
    let mut failed_attempts = 0;
    assert!(!record_line_failure(
        &mut failed_offset,
        &mut failed_attempts,
        0
    ));
    assert!(!record_line_failure(
        &mut failed_offset,
        &mut failed_attempts,
        0
    ));
    assert!(record_line_failure(
        &mut failed_offset,
        &mut failed_attempts,
        0
    ));
    assert_eq!(failed_attempts, 0);
    assert!(!record_line_failure(
        &mut failed_offset,
        &mut failed_attempts,
        10
    ));
}

#[test]
fn skipped_line_detail_records_path_offset_and_attempts() {
    let original = "/logs/output_log_x.txt: parse error";
    let mut pass = LogPass {
        failure: Some(original.to_string()),
        ..LogPass::default()
    };
    pass.record_skip(Path::new("/logs/output_log_x.txt"), 120, 180);

    let detail = pass.failure_detail().expect("failure detail");
    assert!(detail.contains("/logs/output_log_x.txt"), "{detail}");
    assert!(detail.contains("120"), "{detail}");
    assert!(detail.contains("180"), "{detail}");
    assert!(detail.contains("3回失敗したためスキップ"), "{detail}");
    assert!(
        detail.contains("parse error"),
        "the original error is kept: {detail}"
    );
    assert_eq!(pass.skipped, 1);
    assert!(!pass.is_clean());
    assert_ne!(
        error_signature("ログの取り込みに失敗しました", Some(original)),
        error_signature("ログの取り込みに失敗しました", Some(&detail)),
        "the skipped line must change the reported signature"
    );
}

#[test]
fn skipped_line_details_are_capped_and_summarized() {
    let mut pass = LogPass {
        failure: Some("parse error".to_string()),
        ..LogPass::default()
    };
    for index in 0..MAX_SKIPPED_DETAILS + 2 {
        let start = index as i64 * 10;
        pass.record_skip(Path::new("/logs/a.txt"), start, start + 5);
    }

    let detail = pass.failure_detail().expect("failure detail");
    assert_eq!(
        detail.matches("3回失敗したためスキップ").count(),
        MAX_SKIPPED_DETAILS
    );
    assert!(detail.contains("ほか 2 行"), "{detail}");
    assert_eq!(pass.skipped, MAX_SKIPPED_DETAILS + 2);
}

#[test]
fn stale_daemon_errors_clear_after_a_successful_run() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");

    {
        let db = Db::open(&db_path).unwrap();
        db.record_daemon_error(DaemonOperation::Scan, "old scan", None)
            .unwrap();
        db.record_daemon_error(DaemonOperation::Log, "old log", None)
            .unwrap();
        db.record_daemon_error(DaemonOperation::Link, "old link", None)
            .unwrap();
    }
    std::fs::write(
        log_dir.join("output_log_2026-10-21_21-00-00.txt"),
        "2026.10.21 21:23:12 Debug      -  [Behaviour] Unloading scenes\n",
    )
    .unwrap();

    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert!(db.daemon_errors().unwrap().is_empty());
}

#[test]
fn missing_photo_root_records_a_scan_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");

    let mut daemon = Daemon::open(&db_path, Some(missing), Some(log_dir)).unwrap();
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    let errors = db.daemon_errors().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].operation, DaemonOperation::Scan);
    assert!(errors[0].message.contains("写真フォルダ"));
}

#[test]
fn scan_root_success_keeps_another_roots_scan_error() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("photos");
    let missing = dir.path().join("missing");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&good).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    write_photo(&good.join("VRChat_2026-10-21_21-24-00.000_4x3.png"));
    let db_path = dir.path().join("library.db");

    let mut daemon = Daemon::open(&db_path, Some(good.clone()), Some(log_dir)).unwrap();
    daemon.set_ipc_endpoint(None);
    daemon.settings.photo_roots = vec![
        good.to_string_lossy().to_string(),
        missing.to_string_lossy().to_string(),
    ];

    // The full scan covers the missing root and records a Scan error.
    daemon.scan_photos().unwrap();
    {
        let db = daemon.db.lock().unwrap();
        let errors = db.daemon_errors().unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].operation, DaemonOperation::Scan);
    }

    // A successful targeted scan of the healthy root must not clear the
    // error recorded for the missing one.
    let report = daemon.scan_root_photos(good.clone()).unwrap();
    assert!(report.failures.is_empty());
    {
        let db = daemon.db.lock().unwrap();
        let errors = db.daemon_errors().unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].operation, DaemonOperation::Scan);
    }

    // A full scan with every root healthy clears the stale error.
    std::fs::create_dir_all(&missing).unwrap();
    daemon.scan_photos().unwrap();
    let db = daemon.db.lock().unwrap();
    assert!(db.daemon_errors().unwrap().is_empty());
}

#[test]
fn missing_log_folder_records_a_log_error() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    std::fs::create_dir_all(&photo_dir).unwrap();
    let log_dir = dir.path().join("missing-logs");
    let db_path = dir.path().join("library.db");

    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    let errors = db.daemon_errors().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].operation, DaemonOperation::Log);
}

#[test]
fn rescan_command_clears_missing_root_error_and_indexes_the_folder() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir.clone()), Some(log_dir)).unwrap();
    daemon.run_once().unwrap();

    {
        let db = Db::open(&db_path).unwrap();
        assert_eq!(
            db.daemon_errors().unwrap()[0].operation,
            DaemonOperation::Scan
        );
    }

    std::fs::create_dir_all(&photo_dir).unwrap();
    write_photo(&photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png"));
    {
        let db = Db::open(&db_path).unwrap();
        assert!(db.request_daemon_command(DaemonCommand::Rescan).unwrap());
    }
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert!(db.daemon_errors().unwrap().is_empty());
    assert_eq!(db.count_photos().unwrap(), 1);
}

#[test]
fn large_appended_log_is_ingested_in_bounded_passes() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let log_path = dir.path().join("logs/output_log_2026-10-21_21-00-00.txt");
    let join = "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)";
    let name = "2026.10.21 21:23:13 Debug      -  [Behaviour] Joining or Creating Room: Test World";
    let player_one = "2026.10.21 21:23:20 Debug      -  [Behaviour] OnPlayerJoined Mio (usr_a)";
    let player_two = "2026.10.21 21:24:20 Debug      -  [Behaviour] OnPlayerJoined Kenta (usr_b)";
    let filler = "not a vrchat log line";
    let mut head_lines = vec![join.to_string(), name.to_string()];
    for _ in 2..510 {
        head_lines.push(filler.to_string());
    }
    // This player is past the 500-line transaction boundary but still in
    // the first read budget, so session state must carry between chunks.
    head_lines.push(player_one.to_string());
    while head_lines.len() < LOG_READ_MAX_LINES {
        head_lines.push(filler.to_string());
    }
    let head = format!("{}\n", head_lines.join("\n"));
    let tail = format!("{player_two}\n");
    std::fs::write(&log_path, format!("{head}{tail}")).unwrap();

    let log_file = log_path.to_str().unwrap();
    daemon.ingest_logs().unwrap();
    let first_offset = {
        let db = daemon.db.lock().unwrap();
        db.log_offset(log_file).unwrap().unwrap()
    };
    assert_eq!(
        first_offset,
        head.len() as i64,
        "one poll stops at the line budget"
    );
    let joined_at = vrc_photon_core::log_parser::parse_line(join)
        .unwrap()
        .timestamp_utc();
    let player_one_at = vrc_photon_core::log_parser::parse_line(player_one)
        .unwrap()
        .timestamp_utc();
    let player_two_at = vrc_photon_core::log_parser::parse_line(player_two)
        .unwrap()
        .timestamp_utc();
    {
        let db = daemon.db.lock().unwrap();
        let session = db.session_at(joined_at + 1).unwrap().unwrap();
        let players = db.players_at(session.id, player_one_at + 1).unwrap();
        assert_eq!(players.len(), 1);
        assert_eq!(players[0].display_name, "Mio");
        assert_eq!(
            db.players_at(session.id, player_two_at + 1).unwrap().len(),
            1
        );
    }

    daemon.ingest_logs().unwrap();
    {
        let db = daemon.db.lock().unwrap();
        assert_eq!(
            db.log_offset(log_file).unwrap(),
            Some((head.len() + tail.len()) as i64)
        );
        let session = db.session_at(joined_at + 1).unwrap().unwrap();
        let players = db.players_at(session.id, player_two_at + 1).unwrap();
        assert_eq!(players.len(), 2);
        assert_eq!(players[1].display_name, "Kenta");
        assert_eq!(db.open_sessions().unwrap().len(), 1);
    }

    // The next poll has nothing new to ingest and must not create a
    // second session for the same world join.
    daemon.ingest_logs().unwrap();
    let db = daemon.db.lock().unwrap();
    assert_eq!(db.open_sessions().unwrap().len(), 1);
    assert_eq!(
        db.log_offset(log_file).unwrap(),
        Some((head.len() + tail.len()) as i64)
    );
}

#[test]
fn trailing_partial_log_line_is_ingested_after_it_completes() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let log_path = dir.path().join("logs/output_log_2026-10-21_21-00-00.txt");
    let join = "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)";
    let player = "2026.10.21 21:23:20 Debug      -  [Behaviour] OnPlayerJoined Mio (usr_a)";
    std::fs::write(&log_path, format!("{join}\n{player}")).unwrap();
    let log_file = log_path.to_str().unwrap();
    daemon.ingest_logs().unwrap();

    let join_offset = format!("{join}\n").len() as i64;
    let joined_at = vrc_photon_core::log_parser::parse_line(join)
        .unwrap()
        .timestamp_utc();
    let player_at = vrc_photon_core::log_parser::parse_line(player)
        .unwrap()
        .timestamp_utc();
    {
        let db = daemon.db.lock().unwrap();
        assert_eq!(db.log_offset(log_file).unwrap(), Some(join_offset));
        let session = db.session_at(joined_at + 1).unwrap().unwrap();
        assert!(db.players_at(session.id, player_at).unwrap().is_empty());
    }

    let mut file = File::options().append(true).open(&log_path).unwrap();
    writeln!(file).unwrap();
    drop(file);
    daemon.ingest_logs().unwrap();

    let db = daemon.db.lock().unwrap();
    assert_eq!(
        db.log_offset(log_file).unwrap(),
        Some(join_offset + player.len() as i64 + 1)
    );
    let session = db.session_at(joined_at + 1).unwrap().unwrap();
    let players = db.players_at(session.id, player_at + 1).unwrap();
    assert_eq!(players.len(), 1);
    assert_eq!(players[0].display_name, "Mio");
}

#[test]
fn relog_command_clears_a_log_error_and_reads_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    std::fs::create_dir_all(&photo_dir).unwrap();
    let log_dir = dir.path().join("logs");
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir.clone())).unwrap();
    daemon.run_once().unwrap();

    std::fs::create_dir_all(&log_dir).unwrap();
    let log_path = log_dir.join("output_log_2026-10-21_21-00-00.txt");
    std::fs::write(
        &log_path,
        concat!(
            "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)\n",
            "2026.10.21 21:23:13 Debug      -  [Behaviour] Joining or Creating Room: Test World\n",
        ),
    )
    .unwrap();
    {
        let db = Db::open(&db_path).unwrap();
        assert!(db.request_daemon_command(DaemonCommand::Relog).unwrap());
    }
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert!(db.daemon_errors().unwrap().is_empty());
    assert!(db.log_offset(log_path.to_str().unwrap()).unwrap().is_some());
}

#[test]
fn command_reloads_settings_and_rebuilds_watchers() {
    let dir = tempfile::tempdir().unwrap();
    let old_root = dir.path().join("old");
    let new_root = dir.path().join("new");
    std::fs::create_dir_all(&old_root).unwrap();
    std::fs::create_dir_all(&new_root).unwrap();
    write_photo(&new_root.join("VRChat_2026-10-21_21-24-00.000_4x3.png"));
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");

    let mut daemon = Daemon::open(&db_path, Some(old_root), Some(log_dir)).unwrap();
    {
        let db = Db::open(&db_path).unwrap();
        let mut settings = AppSettings::load(&db).unwrap();
        settings.photo_roots = vec![new_root.to_string_lossy().to_string()];
        settings.save(&db).unwrap();
        assert!(db.request_daemon_command(DaemonCommand::Rescan).unwrap());
    }
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert_eq!(db.count_photos().unwrap(), 1);
    assert_eq!(daemon.watchers.len(), 1);
    assert!(db.daemon_errors().unwrap().is_empty());
}

#[test]
fn watcher_roots_include_library_roots_outside_settings_roots() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let imported = dir.path().join("imported");
    std::fs::create_dir_all(&imported).unwrap();
    add_replaced_photo(
        &daemon,
        &imported.join("VRChat_imported.png"),
        &imported.join("compressed/imported.webp"),
    );

    daemon.restart_watchers();

    assert!(daemon.watcher_roots.contains(&imported));
    assert!(daemon.watcher_roots.contains(&dir.path().join("photos")));
}

#[test]
fn poll_watchers_queues_removals_and_ignores_derivative_creates() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.restart_watchers();
    let (tx, rx) = std::sync::mpsc::channel();
    daemon.watcher_rx = Some(rx);
    let photo_dir = dir.path().join("photos");
    let compressed = photo_dir.join("compressed");

    let created = compressed.join("created.webp");
    let removed = compressed.join("removed.webp");
    let old = compressed.join("old.webp");
    let new = compressed.join("new.webp");
    let source = photo_dir.join("VRChat_new.png");

    tx.send(Ok(notify::Event::new(notify::EventKind::Create(
        notify::event::CreateKind::File,
    ))
    .add_path(created.clone())))
        .unwrap();
    tx.send(Ok(notify::Event::new(notify::EventKind::Remove(
        notify::event::RemoveKind::File,
    ))
    .add_path(removed.clone())))
        .unwrap();
    tx.send(Ok(notify::Event::new(notify::EventKind::Modify(
        notify::event::ModifyKind::Name(notify::event::RenameMode::Both),
    ))
    .add_path(old.clone())
    .add_path(new)))
        .unwrap();
    tx.send(Ok(notify::Event::new(notify::EventKind::Remove(
        notify::event::RemoveKind::File,
    ))
    .add_path(source.clone())))
        .unwrap();

    daemon.poll_watchers();

    assert!(!daemon.dirty_paths.contains(&created));
    assert!(daemon.dirty_paths.contains(&removed));
    assert!(daemon.dirty_paths.contains(&old));
    assert!(daemon.dirty_paths.contains(&source));
}

#[test]
fn poll_commands_preserves_buffered_watcher_events() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let photo_dir = dir.path().join("photos");
    // Establish the real watchers so the root set is recorded, then
    // stand in for the channel with an event already buffered.
    daemon.restart_watchers();
    assert_eq!(daemon.watcher_roots, vec![photo_dir.clone()]);
    let (tx, rx) = std::sync::mpsc::channel();
    daemon.watcher_rx = Some(rx);
    let create = |path: PathBuf| -> notify::Result<notify::Event> {
        Ok(
            notify::Event::new(notify::EventKind::Create(notify::event::CreateKind::File))
                .add_path(path),
        )
    };
    let first = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    tx.send(create(first.clone())).unwrap();
    {
        let db = Db::open(dir.path().join("library.db")).unwrap();
        assert!(db.request_daemon_command(DaemonCommand::Relog).unwrap());
    }
    daemon.poll_commands().unwrap();

    // The buffered event was drained, not dropped with a rebuilt
    // channel, and the unchanged roots keep the same channel alive.
    assert_eq!(daemon.dirty_paths, vec![first]);
    let second = photo_dir.join("VRChat_2026-10-21_21-25-00.000_4x3.png");
    tx.send(create(second.clone())).unwrap();
    daemon.poll_watchers();
    assert_eq!(daemon.dirty_paths.len(), 2);
    assert_eq!(daemon.dirty_paths[1], second);
}

#[test]
fn poll_commands_drains_watcher_events_before_a_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let photo_dir = dir.path().join("photos");
    daemon.restart_watchers();
    let (tx, rx) = std::sync::mpsc::channel();
    daemon.watcher_rx = Some(rx);
    let pending = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    tx.send(Ok(notify::Event::new(notify::EventKind::Create(
        notify::event::CreateKind::File,
    ))
    .add_path(pending.clone())))
        .unwrap();
    // Point the settings at a different root so the batch rebuilds.
    let new_root = dir.path().join("new");
    std::fs::create_dir_all(&new_root).unwrap();
    {
        let db = Db::open(dir.path().join("library.db")).unwrap();
        let mut settings = AppSettings::load(&db).unwrap();
        settings.photo_roots = vec![new_root.to_string_lossy().to_string()];
        settings.save(&db).unwrap();
        assert!(db.request_daemon_command(DaemonCommand::Relog).unwrap());
    }
    daemon.poll_commands().unwrap();

    assert_eq!(daemon.dirty_paths, vec![pending]);
    assert_eq!(daemon.watcher_roots, vec![new_root]);
}

#[test]
fn watcher_failure_survives_clean_scans_until_watchers_recover() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    write_photo(&original_path(&dir));
    let db_path = dir.path().join("library.db");
    // Seed the state a failed watcher start leaves behind.
    let watcher_error = LocalizedError::new("daemon.watcher_failed");
    daemon.watcher_error = Some((watcher_error.clone(), Some("boom".to_string())));
    daemon
        .report_localized_error(DaemonOperation::Scan, watcher_error, Some("boom"))
        .unwrap();

    // A clean full scan must not erase the watcher failure.
    daemon.scan_photos().unwrap();
    {
        let db = Db::open(&db_path).unwrap();
        let errors = db.daemon_errors().unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].message_key.as_deref(),
            Some("daemon.watcher_failed")
        );
    }

    // A scan failure takes the slot; the next clean scan clears it and
    // surfaces the still-unrecovered watcher failure again.
    daemon
        .report_localized_error(
            DaemonOperation::Scan,
            LocalizedError::new("daemon.scan_failed").arg("count", 1),
            Some("detail"),
        )
        .unwrap();
    daemon.scan_photos().unwrap();
    {
        let db = Db::open(&db_path).unwrap();
        let errors = db.daemon_errors().unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].message_key.as_deref(),
            Some("daemon.watcher_failed")
        );
    }

    // Once the watchers start, the failure clears.
    daemon.restart_watchers();
    let db = Db::open(&db_path).unwrap();
    assert!(db.daemon_errors().unwrap().is_empty());
}

#[test]
fn run_once_generates_thumbnails_for_command_scans() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let cache = dir.path().join("thumbnails");
    std::fs::create_dir_all(&cache).unwrap();
    daemon.set_thumbnail_dir(cache.clone());
    write_photo(&original_path(&dir));
    {
        let db = Db::open(dir.path().join("library.db")).unwrap();
        assert!(db.request_daemon_command(DaemonCommand::Rescan).unwrap());
    }

    daemon.run_once().unwrap();

    // The Rescan command indexed the photo, so run_once's own scan had
    // nothing new; the thumbnail still exists.
    let db = Db::open(dir.path().join("library.db")).unwrap();
    assert_eq!(db.count_photos().unwrap(), 1);
    assert!(thumbnail::thumbnail_path_in(&cache, 1, daemon.settings().thumbnail_spec()).exists());
}

#[test]
fn scan_marks_missing_only_for_roots_that_scanned_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let healthy = dir.path().join("photos");
    let deleted = healthy.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&deleted);
    // A "root" that is a plain file fails as a root; the photo indexed
    // under it earlier must not be marked missing by the healthy
    // root's clean diff.
    let broken_root = dir.path().join("broken");
    std::fs::write(&broken_root, b"not a directory").unwrap();
    let broken_photo_id = {
        let db = daemon.db.lock().unwrap();
        let root_id = db
            .add_root(&broken_root.to_string_lossy(), "reference")
            .unwrap();
        db.upsert_photo(&vrc_photon_core::model::NewPhoto {
            root_id,
            rel_path: "lost.png".to_string(),
            abs_path: broken_root.join("lost.png").to_string_lossy().to_string(),
            file_name: "lost.png".to_string(),
            captured_at_utc: 1_700_000_000,
            captured_at_local: "2026-10-21 21:24:00".to_string(),
            tz_offset_sec: 0,
            width: 4,
            height: 3,
            file_size: 10,
            mtime: 1,
            blake3: None,
        })
        .unwrap()
    };
    daemon.settings.photo_roots = vec![
        broken_root.to_string_lossy().to_string(),
        healthy.to_string_lossy().to_string(),
    ];

    let first = daemon.scan_photos().unwrap();
    assert_eq!(first.indexed, 1);
    std::fs::remove_file(&deleted).unwrap();
    let report = daemon.scan_photos().unwrap();

    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].path, broken_root);
    let db = daemon.db.lock().unwrap();
    let healthy_photo = db
        .photo_by_path(deleted.to_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(healthy_photo.status, PhotoStatus::Missing);
    let broken_photo = db.photo_by_id(broken_photo_id).unwrap().unwrap();
    assert_eq!(broken_photo.status, PhotoStatus::Present);
}

#[test]
fn poll_commands_rescan_dispatches_scan_without_run_once() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");
    {
        let db = Db::open(&db_path).unwrap();
        db.record_daemon_error(DaemonOperation::Scan, "old scan", None)
            .unwrap();
        assert!(db.request_daemon_command(DaemonCommand::Rescan).unwrap());
    }

    let mut daemon = Daemon::open(&db_path, Some(photo_dir.clone()), Some(log_dir)).unwrap();
    std::fs::create_dir_all(&photo_dir).unwrap();
    write_photo(&photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png"));
    daemon.poll_commands().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert_eq!(db.count_photos().unwrap(), 1);
    assert!(db.daemon_errors().unwrap().is_empty());
}

#[test]
fn poll_commands_relog_dispatches_log_ingestion_without_run_once() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    std::fs::create_dir_all(&photo_dir).unwrap();
    let log_dir = dir.path().join("logs");
    let db_path = dir.path().join("library.db");
    {
        let db = Db::open(&db_path).unwrap();
        db.record_daemon_error(DaemonOperation::Log, "old log", None)
            .unwrap();
        assert!(db.request_daemon_command(DaemonCommand::Relog).unwrap());
    }

    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir.clone())).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let log_path = log_dir.join("output_log_2026-10-21_21-00-00.txt");
    std::fs::write(
        &log_path,
        concat!(
            "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)\n",
            "2026.10.21 21:23:13 Debug      -  [Behaviour] Joining or Creating Room: Test World\n",
            "2026.10.21 21:23:20 Debug      -  [Behaviour] OnPlayerJoined Mio (usr_a)\n",
        ),
    )
    .unwrap();
    daemon.poll_commands().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert!(db.daemon_errors().unwrap().is_empty());
    assert!(db.log_offset(log_path.to_str().unwrap()).unwrap().is_some());
    let joined_at = vrc_photon_core::log_parser::parse_line(
        "2026.10.21 21:23:20 Debug      -  [Behaviour] OnPlayerJoined Mio (usr_a)",
    )
    .unwrap()
    .timestamp_utc();
    let session = db.session_at(joined_at).unwrap().expect("log ingested");
    let world = db.world_by_id(session.world_id).unwrap().unwrap();
    assert_eq!(world.name.as_deref(), Some("Test World"));
    assert_eq!(db.players_at(session.id, joined_at).unwrap().len(), 1);
}

#[test]
fn poll_commands_relink_dispatches_linking_without_run_once() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    std::fs::create_dir_all(&photo_dir).unwrap();
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");
    let photo_path = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&photo_path);

    let photo_id = {
        let db = Db::open(&db_path).unwrap();
        let mut ingestor = LogIngestor::new("output_log_2026-10-21_21-00-00.txt");
        for line in [
            "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)",
            "2026.10.21 21:23:13 Debug      -  [Behaviour] Joining or Creating Room: Test World",
            "2026.10.21 21:23:20 Debug      -  [Behaviour] OnPlayerJoined Mio (usr_a)",
        ] {
            ingestor.ingest_line(&db, line).unwrap();
        }
        let captured_at = vrc_photon_core::log_parser::parse_line(
            "2026.10.21 21:24:00 Debug      -  [Behaviour] Unloading scenes",
        )
        .unwrap()
        .timestamp_utc();
        let root = db
            .add_root(photo_dir.to_str().unwrap(), "reference")
            .unwrap();
        let file_name = photo_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let photo_id = db
            .upsert_photo(&vrc_photon_core::model::NewPhoto {
                root_id: root,
                rel_path: file_name.clone(),
                abs_path: photo_path.to_string_lossy().to_string(),
                file_name,
                captured_at_utc: captured_at,
                captured_at_local: "2026-10-21 21:24:00.000".to_string(),
                tz_offset_sec: 9 * 3600,
                width: 16,
                height: 10,
                file_size: 1,
                mtime: captured_at,
                blake3: None,
            })
            .unwrap();
        assert!(db.photo_context(photo_id).unwrap().is_none());
        assert!(db.request_daemon_command(DaemonCommand::Relink).unwrap());
        photo_id
    };

    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.poll_commands().unwrap();

    let db = Db::open(&db_path).unwrap();
    let context = db
        .photo_context(photo_id)
        .unwrap()
        .expect("relink linked the photo");
    assert_eq!(context.world_name.as_deref(), Some("Test World"));
    let players = db.photo_players(photo_id).unwrap();
    assert_eq!(players.len(), 1);
    assert_eq!(players[0].display_name, "Mio");
}

#[test]
fn xmp_read_happens_without_the_db_lock() {
    use std::sync::Barrier;

    let db = Arc::new(Mutex::new(Db::open_in_memory().unwrap()));
    let photo = {
        let db = lock_db(&db);
        let root = db.add_root("/photos", "reference").unwrap();
        let id = db
            .upsert_photo(&NewPhoto {
                root_id: root,
                rel_path: "a.png".to_string(),
                abs_path: "/photos/a.png".to_string(),
                file_name: "a.png".to_string(),
                captured_at_utc: 1,
                captured_at_local: "2026-11-11 12:00:00".to_string(),
                tz_offset_sec: 0,
                width: 16,
                height: 10,
                file_size: 1,
                mtime: 1,
                blake3: None,
            })
            .unwrap();
        db.photo_by_id(id).unwrap().unwrap()
    };

    let start = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let worker_db = Arc::clone(&db);
    let worker_start = Arc::clone(&start);
    let worker_release = Arc::clone(&release);
    let worker = std::thread::spawn(move || {
        relink_photos_outside_lock(&worker_db, vec![photo], false, 0, move |_| {
            worker_start.wait();
            worker_release.wait();
            None
        })
    });

    start.wait();
    let acquired = db.try_lock().is_ok();
    release.wait();
    assert!(acquired, "reading XMP must not hold the database lock");
    assert_eq!(worker.join().unwrap().unwrap(), 0);
}

#[test]
fn rescan_command_retries_a_photo_in_link_backoff() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");
    let captured_at = vrc_photon_core::log_parser::parse_line(
        "2026.10.21 21:24:00 Debug      -  [Behaviour] Unloading scenes",
    )
    .unwrap()
    .timestamp_utc();

    let photo_id = {
        let db = Db::open(&db_path).unwrap();
        let mut ingestor = LogIngestor::new("output_log_2026-10-21_21-00-00.txt");
        for line in [
            "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)",
            "2026.10.21 21:23:13 Debug      -  [Behaviour] Joining or Creating Room: Test World",
        ] {
            ingestor.ingest_line(&db, line).unwrap();
        }
        let photo_id = seed_photo_with_link_backoff(&db, &photo_dir, captured_at);
        assert!(db.photo_context(photo_id).unwrap().is_none());
        assert!(db.request_daemon_command(DaemonCommand::Rescan).unwrap());
        photo_id
    };

    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.poll_commands().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert!(db.photo_context(photo_id).unwrap().is_some());
}

#[test]
fn relog_command_retries_a_photo_in_link_backoff() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");
    let captured_at = vrc_photon_core::log_parser::parse_line(
        "2026.10.21 21:24:00 Debug      -  [Behaviour] Unloading scenes",
    )
    .unwrap()
    .timestamp_utc();

    let photo_id = {
        let db = Db::open(&db_path).unwrap();
        let photo_id = seed_photo_with_link_backoff(&db, &photo_dir, captured_at);
        assert!(db.photo_context(photo_id).unwrap().is_none());
        assert!(db.request_daemon_command(DaemonCommand::Relog).unwrap());
        photo_id
    };
    std::fs::write(
        log_dir.join("output_log_2026-10-21_21-00-00.txt"),
        concat!(
            "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)\n",
            "2026.10.21 21:23:13 Debug      -  [Behaviour] Joining or Creating Room: Test World\n",
        ),
    )
    .unwrap();

    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.poll_commands().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert!(db.photo_context(photo_id).unwrap().is_some());
}

#[test]
fn run_once_consumes_queued_commands() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    {
        let db = Db::open(&db_path).unwrap();
        assert!(db.request_daemon_command(DaemonCommand::Rescan).unwrap());
    }

    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert!(db.take_daemon_commands().unwrap().is_empty());
}

#[test]
fn failed_job_can_be_retried_and_completed() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let photo_path = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&photo_path);
    let db_path = dir.path().join("library.db");
    {
        let db = Db::open(&db_path).unwrap();
        db.save_profile(&CompressionProfile {
            id: "p".to_string(),
            name: "retry".to_string(),
            format: vrc_photon_core::model::CompressionFormat::Webp,
            suffix: "_retry".to_string(),
            ..CompressionProfile::default()
        })
        .unwrap();
    }

    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    daemon.run_once().unwrap();

    let job = {
        let db = Db::open(&db_path).unwrap();
        let photo = db
            .photo_by_path(photo_path.to_str().unwrap())
            .unwrap()
            .unwrap();
        let job = db.enqueue_job(photo.id, "p", "keep").unwrap();
        db.update_job(&job, QueueStatus::Failed, 0.0, Some("boom"), None)
            .unwrap();
        assert!(db.retry_job(&job).unwrap());
        job
    };
    daemon.compress_pending().unwrap();

    let db = Db::open(&db_path).unwrap();
    let job = db
        .list_jobs(10)
        .unwrap()
        .into_iter()
        .find(|row| row.id == job)
        .unwrap();
    assert_eq!(job.status, QueueStatus::Completed);
    assert!(db.derivatives_for_photo(job.photo_id).unwrap().len() == 1);
}

#[cfg(unix)]
#[test]
fn scan_failure_does_not_mark_unseen_photos_missing() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let photo_path = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&photo_path);
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir.clone()), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db
        .photo_by_path(photo_path.to_str().unwrap())
        .unwrap()
        .unwrap();
    drop(db);

    let locked = photo_dir.join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_dir(&locked).is_ok() {
        // Privileged environments bypass directory permissions, so the
        // unreadable-subtree failure cannot be simulated here.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir(&locked).unwrap();
        eprintln!("skipping permission lock: current user can read mode 000");
        return;
    }
    std::fs::remove_file(&photo_path).unwrap();
    daemon.run_once().unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db.photo_by_id(photo.id).unwrap().unwrap();
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Present);
}

#[test]
fn cancelled_scan_does_not_mark_unseen_photos_missing() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let photo_path = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&photo_path);
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db
        .photo_by_path(photo_path.to_str().unwrap())
        .unwrap()
        .unwrap();
    drop(db);

    std::fs::remove_file(&photo_path).unwrap();
    daemon.running.store(false, Ordering::Relaxed);
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db.photo_by_id(photo.id).unwrap().unwrap();
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Present);
}

#[cfg(unix)]
#[test]
fn unstatable_photo_keeps_the_existing_row_present() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let locked = photo_dir.join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&log_dir).unwrap();
    let photo_path = locked.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&photo_path);
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    daemon.run_once().unwrap();

    let photo = {
        let db = Db::open(&db_path).unwrap();
        db.photo_by_path(photo_path.to_str().unwrap())
            .unwrap()
            .expect("photo indexed")
    };

    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o444)).unwrap();
    let child_type = std::fs::read_dir(&locked)
        .ok()
        .and_then(|mut entries| entries.next())
        .and_then(Result::ok)
        .and_then(|entry| entry.file_type().ok());
    if std::fs::metadata(&photo_path).is_ok() || !child_type.is_some_and(|kind| kind.is_file()) {
        // Root and some filesystems ignore the missing execute bit, so the
        // stat failure cannot be simulated here.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("skipping stat lock: cannot simulate a listable but unstatable directory");
        return;
    }

    daemon.run_once().unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db.photo_by_id(photo.id).unwrap().unwrap();
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Present);
    let errors = db.daemon_errors().unwrap();
    assert_eq!(errors.len(), 1, "errors: {errors:?}");
    assert_eq!(errors[0].operation, DaemonOperation::Scan);
    assert!(
        errors[0]
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("VRChat_2026-10-21_21-24-00.000_4x3.png"),
        "detail should name the unreadable photo: {:?}",
        errors[0].detail
    );
}

#[test]
fn end_to_end_scan_log_and_link() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();

    let photo_path = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    image::RgbaImage::from_pixel(4, 3, image::Rgba([10, 20, 30, 255]))
        .save(&photo_path)
        .unwrap();

    let log_path = log_dir.join("output_log_2026-10-21_21-00-00.txt");
    let log = concat!(
        "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)\n",
        "2026.10.21 21:23:13 Debug      -  [Behaviour] Joining or Creating Room: Test World\n",
        "2026.10.21 21:23:20 Debug      -  [Behaviour] OnPlayerJoined Mio (usr_a)\n",
    );
    std::fs::write(&log_path, log).unwrap();

    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db
        .photo_by_path(photo_path.to_str().unwrap())
        .unwrap()
        .expect("photo indexed");
    let context = db.photo_context(photo.id).unwrap().expect("photo linked");
    assert_eq!(context.world_name.as_deref(), Some("Test World"));
    let players = db.photo_players(photo.id).unwrap();
    assert_eq!(players.len(), 1);
    assert_eq!(players[0].display_name, "Mio");

    std::fs::remove_file(&photo_path).unwrap();
    daemon.run_once().unwrap();
    let photo = db.photo_by_id(photo.id).unwrap().unwrap();
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Missing);
}

#[test]
fn startup_repairs_open_sessions_at_the_log_mtime() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let log_path = log_dir.join("output_log_2026-10-21_21-00-00.txt");
    std::fs::write(&log_path, "").unwrap();
    let mtime: i64 = 1_700_000_500;
    let file = File::options().write(true).open(&log_path).unwrap();
    file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(mtime as u64))
        .unwrap();

    let db_path = dir.path().join("library.db");
    {
        let db = Db::open(&db_path).unwrap();
        let world = db
            .upsert_world("wrld_test", Some("Test World"), Some("log"))
            .unwrap();
        let session = db
            .create_session(log_path.to_str().unwrap(), mtime - 120, world, None)
            .unwrap();
        db.upsert_session_player(session, "usr_a", "Mio", mtime - 60)
            .unwrap();
    }

    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    // Inject the process state so this test is deterministic on a machine
    // that happens to have VRChat running.
    assert_eq!(
        daemon
            .repair_open_sessions_with_running(Some(false))
            .unwrap(),
        1
    );

    let db = Db::open(&db_path).unwrap();
    assert!(db.open_sessions().unwrap().is_empty());
    let session = db.session_at(mtime - 60).unwrap().unwrap();
    assert_eq!(session.ended_at, Some(mtime));
}

#[test]
fn resume_decision_requires_the_latest_log_and_a_live_vrchat() {
    let latest = Path::new("/logs/output_log_2026-10-21_21-00-00.txt");

    assert!(should_resume_open_session(latest, Some(latest), Some(true)));
    assert!(!should_resume_open_session(
        Path::new("/logs/output_log_2026-10-20_21-00-00.txt"),
        Some(latest),
        Some(true)
    ));
    assert!(!should_resume_open_session(
        latest,
        Some(latest),
        Some(false)
    ));
    assert!(!should_resume_open_session(latest, Some(latest), None));
    assert!(!should_resume_open_session(latest, None, Some(true)));
}

#[test]
fn restart_mid_session_resumes_and_applies_a_later_player_leave() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("logs/output_log_2026-10-21_21-00-00.txt");
    let join = "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)";
    let name = "2026.10.21 21:23:13 Debug      -  [Behaviour] Joining or Creating Room: Test World";
    let player = "2026.10.21 21:23:20 Debug      -  [Behaviour] OnPlayerJoined Mio (usr_a)";
    let leave = "2026.10.21 21:25:00 Debug      -  [Behaviour] OnPlayerLeft Mio Renamed (usr_a)";

    let mut first = test_daemon(&dir);
    std::fs::write(&log_path, format!("{join}\n{name}\n{player}\n")).unwrap();
    let log_file = log_path.to_str().unwrap();
    first.ingest_logs().unwrap();
    let session_id = {
        let db = first.db.lock().unwrap();
        let session = db
            .session_at(
                vrc_photon_core::log_parser::parse_line(join)
                    .unwrap()
                    .timestamp_utc()
                    + 1,
            )
            .unwrap()
            .unwrap();
        assert_eq!(db.open_sessions().unwrap().len(), 1);
        session.id
    };
    let offset_before_restart = {
        let db = first.db.lock().unwrap();
        db.log_offset(log_file).unwrap().unwrap()
    };
    // Simulate a crash: dropping the daemon must not close the session.
    drop(first);

    let mut file = File::options().append(true).open(&log_path).unwrap();
    writeln!(file, "{leave}").unwrap();
    drop(file);

    let mut second = test_daemon(&dir);
    assert_eq!(
        second
            .repair_open_sessions_with_running(Some(true))
            .unwrap(),
        0,
        "the live session must be kept open"
    );
    assert_eq!(
        second.db.lock().unwrap().open_sessions().unwrap()[0].id,
        session_id
    );
    assert!(second.log_tail.is_none(), "repair must not replay the log");

    second.ingest_logs().unwrap();

    let left_at = vrc_photon_core::log_parser::parse_line(leave)
        .unwrap()
        .timestamp_utc();
    let db = second.db.lock().unwrap();
    let players = db.players_at(session_id, left_at - 1).unwrap();
    assert_eq!(players.len(), 1);
    assert_eq!(players[0].display_name, "Mio Renamed");
    assert_eq!(players[0].left_at, Some(left_at));
    assert_eq!(db.open_sessions().unwrap()[0].id, session_id);
    assert_eq!(
        db.log_offset(log_file).unwrap(),
        Some(offset_before_restart + leave.len() as i64 + 1),
        "ingestion resumes after the persisted offset"
    );
    assert_eq!(
        second.log_tail.as_ref().unwrap().ingestor.session_id(),
        Some(session_id)
    );
}

#[test]
fn startup_repair_falls_back_to_the_ingested_log_mtime() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let missing_log = log_dir.join("output_log_deleted.txt");
    let db_path = dir.path().join("library.db");
    {
        let db = Db::open(&db_path).unwrap();
        let world = db.upsert_world("wrld_test", None, None).unwrap();
        db.create_session(missing_log.to_str().unwrap(), 100, world, None)
            .unwrap();
        db.set_log_offset(missing_log.to_str().unwrap(), 10, 500, 10, "1")
            .unwrap();
    }

    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    assert_eq!(daemon.repair_open_sessions().unwrap(), 1);

    let db = Db::open(&db_path).unwrap();
    let session = db.session_at(200).unwrap().unwrap();
    assert_eq!(session.ended_at, Some(500));
}

#[test]
fn maintenance_run_does_not_repair_open_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");
    {
        let db = Db::open(&db_path).unwrap();
        let world = db.upsert_world("wrld_test", None, None).unwrap();
        db.create_session("output_log_live.txt", 100, world, None)
            .unwrap();
    }

    let mut daemon =
        Daemon::open_preserving_running_deletions(&db_path, Some(photo_dir), Some(log_dir))
            .unwrap();
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert_eq!(db.open_sessions().unwrap().len(), 1);
}

#[test]
fn vrchat_exit_closes_the_session_at_the_log_mtime_after_consecutive_misses() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let log_path = dir.path().join("logs/output_log_2026-10-21_21-00-00.txt");
    let join = "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)";
    let name = "2026.10.21 21:23:13 Debug      -  [Behaviour] Joining or Creating Room: Test World";
    std::fs::write(&log_path, format!("{join}\n{name}\n")).unwrap();
    daemon.ingest_logs().unwrap();
    let started = vrc_photon_core::log_parser::parse_line(join)
        .unwrap()
        .timestamp_utc();
    let mtime = started + 120;
    let file = File::options().write(true).open(&log_path).unwrap();
    file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(mtime as u64))
        .unwrap();
    assert!(
        daemon
            .log_tail
            .as_ref()
            .unwrap()
            .ingestor
            .session_id()
            .is_some()
    );

    daemon.observe_vrchat_presence(Some(false)).unwrap();
    daemon.observe_vrchat_presence(Some(false)).unwrap();
    daemon.observe_vrchat_presence(Some(true)).unwrap();
    daemon.observe_vrchat_presence(Some(false)).unwrap();
    daemon.observe_vrchat_presence(Some(false)).unwrap();
    {
        let db = daemon.db.lock().unwrap();
        assert!(db.session_at(started + 1).unwrap().is_some());
    }
    daemon.observe_vrchat_presence(Some(false)).unwrap();

    assert!(
        daemon
            .log_tail
            .as_ref()
            .unwrap()
            .ingestor
            .session_id()
            .is_none()
    );
    let db = daemon.db.lock().unwrap();
    let session = db.session_at(started + 1).unwrap().unwrap();
    assert_eq!(session.ended_at, Some(mtime));
    drop(db);
    let status = daemon.status.lock().unwrap();
    assert_eq!(status.world_name, None);
}

#[test]
fn photos_imported_after_vrchat_exit_do_not_link_to_the_closed_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let log_path = dir.path().join("logs/output_log_2026-10-21_21-00-00.txt");
    let join = "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)";
    let name = "2026.10.21 21:23:13 Debug      -  [Behaviour] Joining or Creating Room: Test World";
    std::fs::write(&log_path, format!("{join}\n{name}\n")).unwrap();
    daemon.ingest_logs().unwrap();
    let started = vrc_photon_core::log_parser::parse_line(join)
        .unwrap()
        .timestamp_utc();
    let mtime = started + 120;
    let file = File::options().write(true).open(&log_path).unwrap();
    file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(mtime as u64))
        .unwrap();
    for _ in 0..VRCHAT_MISSING_POLLS_BEFORE_CLOSE {
        daemon.observe_vrchat_presence(Some(false)).unwrap();
    }

    let photo_path = dir
        .path()
        .join("photos/VRChat_2026-10-21_21-26-00.000_4x3.png");
    write_photo(&photo_path);
    daemon.run_once().unwrap();

    let db = daemon.db.lock().unwrap();
    let photo = db
        .photo_by_path(photo_path.to_str().unwrap())
        .unwrap()
        .expect("photo indexed");
    assert!(db.photo_context(photo.id).unwrap().is_none());
}

#[test]
fn runtime_close_uses_the_ingested_mtime_when_the_log_file_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let log_path = dir.path().join("logs/output_log_2026-10-21_21-00-00.txt");
    let join = "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)";
    std::fs::write(&log_path, format!("{join}\n")).unwrap();
    let started = vrc_photon_core::log_parser::parse_line(join)
        .unwrap()
        .timestamp_utc();
    let mtime = started + 60;
    let file = File::options().write(true).open(&log_path).unwrap();
    file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(mtime as u64))
        .unwrap();
    daemon.ingest_logs().unwrap();
    std::fs::remove_file(&log_path).unwrap();

    for _ in 0..VRCHAT_MISSING_POLLS_BEFORE_CLOSE {
        daemon.observe_vrchat_presence(Some(false)).unwrap();
    }

    let db = daemon.db.lock().unwrap();
    let session = db.session_at(started + 1).unwrap().unwrap();
    assert_eq!(session.ended_at, Some(mtime));
}

#[test]
fn shutdown_uses_the_ingested_mtime_when_the_log_file_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let log_path = dir.path().join("logs/output_log_2026-10-21_21-00-00.txt");
    let join = "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)";
    std::fs::write(&log_path, format!("{join}\n")).unwrap();
    let started = vrc_photon_core::log_parser::parse_line(join)
        .unwrap()
        .timestamp_utc();
    let mtime = started + 60;
    let file = File::options().write(true).open(&log_path).unwrap();
    file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(mtime as u64))
        .unwrap();
    daemon.ingest_logs().unwrap();
    std::fs::remove_file(&log_path).unwrap();

    daemon.shutdown();

    let db = daemon.db.lock().unwrap();
    let session = db.session_at(started + 1).unwrap().unwrap();
    assert_eq!(session.ended_at, Some(mtime));
}

#[test]
fn future_log_mtime_is_clamped_to_now() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let log_path = dir.path().join("logs/output_log_2026-10-21_21-00-00.txt");
    let join = "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)";
    std::fs::write(&log_path, format!("{join}\n")).unwrap();
    daemon.ingest_logs().unwrap();
    let started = vrc_photon_core::log_parser::parse_line(join)
        .unwrap()
        .timestamp_utc();
    let now = chrono::Utc::now().timestamp();
    let future = now + 3600;
    let file = File::options().write(true).open(&log_path).unwrap();
    file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(future as u64))
        .unwrap();

    for _ in 0..VRCHAT_MISSING_POLLS_BEFORE_CLOSE {
        daemon.observe_vrchat_presence(Some(false)).unwrap();
    }

    let db = daemon.db.lock().unwrap();
    let session = db.session_at(started + 1).unwrap().unwrap();
    assert!(session.ended_at.unwrap() <= chrono::Utc::now().timestamp());
    assert!(db.session_at(now + 1).unwrap().is_none());
}

#[test]
fn pending_misses_do_not_close_a_new_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let log_path = dir.path().join("logs/output_log_2026-10-21_21-00-00.txt");
    let first_join =
        "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_first:12345~region(jp)";
    std::fs::write(&log_path, format!("{first_join}\n")).unwrap();
    daemon.ingest_logs().unwrap();
    daemon.observe_vrchat_presence(Some(false)).unwrap();
    daemon.observe_vrchat_presence(Some(false)).unwrap();

    let second_join =
        "2026.10.21 21:24:12 Debug      -  [Behaviour] Joining wrld_second:12345~region(jp)";
    let mut file = File::options().append(true).open(&log_path).unwrap();
    writeln!(file, "{second_join}").unwrap();
    daemon.ingest_logs().unwrap();
    daemon.observe_vrchat_presence(Some(false)).unwrap();

    let second_started = vrc_photon_core::log_parser::parse_line(second_join)
        .unwrap()
        .timestamp_utc();
    let db = daemon.db.lock().unwrap();
    assert!(
        db.session_at(second_started + 1)
            .unwrap()
            .unwrap()
            .ended_at
            .is_none()
    );
}

#[test]
fn unknown_vrchat_presence_does_not_count_as_a_miss() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let log_path = dir.path().join("logs/output_log_2026-10-21_21-00-00.txt");
    let join = "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)";
    std::fs::write(&log_path, format!("{join}\n")).unwrap();
    daemon.ingest_logs().unwrap();

    for _ in 0..10 {
        daemon.observe_vrchat_presence(None).unwrap();
    }

    daemon.observe_vrchat_presence(Some(false)).unwrap();
    daemon.observe_vrchat_presence(Some(false)).unwrap();
    daemon.observe_vrchat_presence(None).unwrap();
    daemon.observe_vrchat_presence(Some(false)).unwrap();

    let db = daemon.db.lock().unwrap();
    let started = vrc_photon_core::log_parser::parse_line(join)
        .unwrap()
        .timestamp_utc();
    assert!(
        db.session_at(started + 1)
            .unwrap()
            .unwrap()
            .ended_at
            .is_none()
    );
}

#[test]
fn shutdown_closes_the_session_at_the_log_mtime() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let log_path = dir.path().join("logs/output_log_2026-10-21_21-00-00.txt");
    let join = "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)";
    std::fs::write(&log_path, format!("{join}\n")).unwrap();
    daemon.ingest_logs().unwrap();
    let started = vrc_photon_core::log_parser::parse_line(join)
        .unwrap()
        .timestamp_utc();
    let mtime = started + 120;
    let file = File::options().write(true).open(&log_path).unwrap();
    file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(mtime as u64))
        .unwrap();

    daemon.shutdown();

    let db = daemon.db.lock().unwrap();
    let session = db.session_at(started + 1).unwrap().unwrap();
    assert_eq!(session.ended_at, Some(mtime));
}

#[test]
fn session_end_time_handles_a_start_time_in_the_future() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("library.db")).unwrap();
    let ended_at = session_end_time(&db, Path::new("/missing/log.txt"), 2_000, 1_000);
    assert_eq!(ended_at, 1_000);
}

fn save_daemon_settings(daemon: &Daemon, update: impl FnOnce(&mut AppSettings)) {
    let db = daemon.db.lock().unwrap();
    let mut settings = AppSettings::load(&db).unwrap_or_default();
    update(&mut settings);
    settings.save(&db).unwrap();
}

fn write_photo(path: &Path) {
    image::RgbaImage::from_pixel(16, 10, image::Rgba([10, 20, 30, 255]))
        .save(path)
        .unwrap();
}

/// Stores a photo that is on disk but has a future link retry time, as if
/// a previous batch failed to find its session.
fn seed_photo_with_link_backoff(db: &Db, photo_dir: &Path, captured_at: i64) -> i64 {
    let photo_path = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&photo_path);
    let root = db
        .add_root(photo_dir.to_str().unwrap(), "reference")
        .unwrap();
    let file_name = photo_path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let photo_id = db
        .upsert_photo(&NewPhoto {
            root_id: root,
            rel_path: file_name.clone(),
            abs_path: photo_path.to_string_lossy().to_string(),
            file_name,
            captured_at_utc: captured_at,
            captured_at_local: "2026-10-21 21:24:00.000".to_string(),
            tz_offset_sec: 9 * 3600,
            width: 16,
            height: 10,
            file_size: 1,
            mtime: captured_at,
            blake3: None,
        })
        .unwrap();
    db.record_link_attempt(photo_id, chrono::Utc::now().timestamp())
        .unwrap();
    photo_id
}

fn test_daemon(dir: &tempfile::TempDir) -> Daemon {
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.set_ipc_endpoint(None);
    daemon
}

/// Adds a replaced photo whose display asset lives at `derivative`. The
/// original and derivative files do not have to exist; callers decide what
/// the verification pass should see on disk.
fn add_replaced_photo(daemon: &Daemon, photo: &Path, derivative: &Path) -> i64 {
    let db = daemon.db.lock().unwrap();
    let root = db
        .add_root(
            photo.parent().unwrap().to_string_lossy().as_ref(),
            "reference",
        )
        .unwrap();
    let file_name = photo.file_name().unwrap().to_string_lossy().to_string();
    let id = db
        .upsert_photo(&NewPhoto {
            root_id: root,
            rel_path: file_name.clone(),
            abs_path: photo.to_string_lossy().to_string(),
            file_name,
            captured_at_utc: 100,
            captured_at_local: "2026-10-21 21:24:00.000".to_string(),
            tz_offset_sec: 9 * 3600,
            width: 16,
            height: 10,
            file_size: 1,
            mtime: 100,
            blake3: None,
        })
        .unwrap();
    let derivative = derivative.to_string_lossy().to_string();
    let derivative_id = db
        .add_derivative(
            id,
            &derivative,
            "webp",
            16,
            10,
            1,
            Some("test"),
            CompressionOrigin::Manual,
            "replace",
        )
        .unwrap();
    assert!(db.set_replaced(id, derivative_id).unwrap());
    id
}

#[test]
fn daemon_ai_plugin_service_uses_injected_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let ai_dir = dir.path().join("ai");

    daemon.set_ai_data_dir(ai_dir);
    daemon.set_ai_plugin_keys(vec!["test-key".to_string()]);
    daemon.set_ai_http_client(Arc::new(UreqModelClient::new()));

    assert!(!daemon.plugin_service().is_ready("vrc-photon-autotag"));
}

const PLUGIN: &str = AI_PLUGIN_ID;

/// A daemon with an isolated library and a test plugin that can be marked
/// ready without spawning a sidecar.
struct DaemonAiTest {
    _dir: tempfile::TempDir,
    daemon: Daemon,
    db: Arc<Mutex<Db>>,
    photo_dir: PathBuf,
    photo_seq: usize,
}

impl DaemonAiTest {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let photo_dir = dir.path().join("photos");
        let log_dir = dir.path().join("logs");
        std::fs::create_dir_all(&photo_dir).unwrap();
        std::fs::create_dir_all(&log_dir).unwrap();
        let db_path = dir.path().join("library.db");
        let mut daemon = Daemon::open(&db_path, Some(photo_dir.clone()), Some(log_dir)).unwrap();
        daemon.set_ipc_endpoint(None);
        daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
        daemon.set_ai_data_dir(dir.path().join("ai"));
        // These tests focus on tagging and deletion; automatic compression is
        // covered separately and would otherwise leave pending jobs that hold
        // deletion requests open.
        save_daemon_settings(&daemon, |settings| settings.auto_compress = false);
        let db = daemon.db.clone();
        Self {
            _dir: dir,
            daemon,
            db,
            photo_dir,
            photo_seq: 0,
        }
    }

    fn enable_ai(&mut self, enabled: bool, auto: bool) {
        save_daemon_settings(&self.daemon, |settings| {
            settings.ai_tagging_enabled = enabled;
            settings.ai_tagging_auto = auto;
        });
        self.daemon.settings.ai_tagging_enabled = enabled;
        self.daemon.settings.ai_tagging_auto = auto;
    }

    /// Stores the install, model state, and probe rows `is_ready` checks.
    /// The model files and sidecar are intentionally not created: the
    /// auto-enqueue paths only consult readiness.
    fn mark_plugin_ready(&self) {
        let manifest = sample_manifest();
        let now = chrono::Utc::now().to_rfc3339();
        let install = PluginInstallation {
            plugin_id: manifest.plugin_id.clone(),
            variant: manifest.variant.clone(),
            version: manifest.plugin_version.clone(),
            protocol_version: manifest.protocol_version,
            path: self
                ._dir
                .path()
                .join("plugin")
                .to_string_lossy()
                .into_owned(),
            manifest_json: serde_json::to_string(&manifest).unwrap(),
            catalog_version: Some(manifest.catalog_version.clone()),
            active: true,
            installed_at: now.clone(),
            updated_at: now,
        };
        let probe = Probe {
            protocol_version: manifest.protocol_version,
            sidecar_version: "1.0.0".to_string(),
            plugin_id: manifest.plugin_id.clone(),
            variant: manifest.variant.clone(),
            catalog_version: manifest.catalog_version.clone(),
            backends: Vec::new(),
        };
        {
            let db = self.db.lock().unwrap();
            db.upsert_plugin_installation(&install).unwrap();
            db.set_plugin_model_state(
                PLUGIN,
                vrc_photon_core::ai::ModelState::Ready,
                Some(&manifest.model.revision),
                0,
                None,
                None,
            )
            .unwrap();
            db.set_plugin_probe_json(PLUGIN, Some(&serde_json::to_string(&probe).unwrap()))
                .unwrap();
        }
        assert!(self.daemon.plugin_service().is_ready(PLUGIN));
    }

    fn add_photo(&mut self) -> PathBuf {
        self.photo_seq += 1;
        let path = self.photo_dir.join(format!(
            "VRChat_2026-10-21_21-{:02}-00.000_4x3.png",
            self.photo_seq
        ));
        write_photo(&path);
        path
    }

    fn scan(&mut self) -> ScanReport {
        self.daemon
            .scan_root_photos(self.photo_dir.clone())
            .unwrap()
    }

    fn add_photo_and_scan(&mut self) -> (i64, PathBuf) {
        let path = self.add_photo();
        let report = self.scan();
        assert_eq!(report.new_ids.len(), 1, "one new photo per call");
        (report.new_ids[0], path)
    }

    fn photo_id(&self, path: &Path) -> i64 {
        self.db
            .lock()
            .unwrap()
            .photo_by_path(path.to_str().unwrap())
            .unwrap()
            .unwrap()
            .id
    }

    fn tagging_counts(&self) -> JobStatusCounts {
        self.db.lock().unwrap().tagging_job_counts(PLUGIN).unwrap()
    }
}

#[test]
fn auto_tagging_enqueues_only_new_photos_when_ready() {
    let mut ctx = DaemonAiTest::new();
    ctx.enable_ai(true, true);
    ctx.mark_plugin_ready();

    let (new_id, _) = ctx.add_photo_and_scan();
    assert_eq!(ctx.tagging_counts().pending, 1);
    assert_eq!(
        ctx.db
            .lock()
            .unwrap()
            .tagging_jobs_for_photo(new_id)
            .unwrap()
            .len(),
        1
    );

    ctx.add_photo_and_scan();
    assert_eq!(ctx.tagging_counts().pending, 2);

    // Rescanning the same photos must not duplicate their active jobs.
    ctx.scan();
    assert_eq!(ctx.tagging_counts().pending, 2);
    assert_eq!(
        ctx.db
            .lock()
            .unwrap()
            .tagging_jobs_for_photo(new_id)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn auto_tagging_does_nothing_when_not_ready() {
    let mut ctx = DaemonAiTest::new();
    ctx.enable_ai(true, true);

    ctx.add_photo_and_scan();
    assert_eq!(ctx.tagging_counts().unfinished(), 0);

    ctx.daemon.refresh_status().unwrap();
    assert!(!ctx.daemon.status.lock().unwrap().tagging_ready);
}

#[test]
fn auto_tagging_requires_enabled_and_auto_switches() {
    let mut ctx = DaemonAiTest::new();
    ctx.mark_plugin_ready();

    ctx.enable_ai(false, true);
    ctx.add_photo_and_scan();
    assert_eq!(ctx.tagging_counts().unfinished(), 0);

    ctx.enable_ai(true, false);
    ctx.add_photo_and_scan();
    assert_eq!(ctx.tagging_counts().unfinished(), 0);

    ctx.enable_ai(true, true);
    ctx.add_photo_and_scan();
    assert_eq!(ctx.tagging_counts().pending, 1);
}

#[test]
fn watcher_new_photo_enqueues_tagging() {
    let mut ctx = DaemonAiTest::new();
    ctx.enable_ai(true, true);
    ctx.mark_plugin_ready();
    let path = ctx.add_photo();
    ctx.daemon.dirty_paths.push(path.clone());

    ctx.daemon.index_dirty_paths().unwrap();

    let photo_id = ctx.photo_id(&path);
    assert_eq!(ctx.tagging_counts().pending, 1);
    assert_eq!(
        ctx.db
            .lock()
            .unwrap()
            .tagging_jobs_for_photo(photo_id)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn backlog_enqueue_is_waiting_until_released() {
    let mut ctx = DaemonAiTest::new();
    ctx.add_photo_and_scan();
    ctx.add_photo_and_scan();
    assert_eq!(
        ctx.db
            .lock()
            .unwrap()
            .enqueue_tagging_backlog(PLUGIN)
            .unwrap(),
        2
    );
    assert_eq!(ctx.tagging_counts().waiting, 2);
    assert_eq!(ctx.tagging_counts().pending, 0);

    assert_eq!(ctx.daemon.release_ai_backlog().unwrap(), 2);
    let counts = ctx.tagging_counts();
    assert_eq!(counts.waiting, 0);
    assert_eq!(counts.pending, 2);
}

#[test]
fn release_backlog_wakes_the_worker() {
    let mut ctx = DaemonAiTest::new();
    let (photo_id, _) = ctx.add_photo_and_scan();
    ctx.db
        .lock()
        .unwrap()
        .enqueue_tagging_jobs(&[photo_id], PLUGIN, TaggingJobStatus::Waiting)
        .unwrap();
    let before = ctx.daemon.tagging.generation();

    assert_eq!(ctx.daemon.release_ai_backlog().unwrap(), 1);
    assert!(
        ctx.daemon.tagging.generation() > before,
        "release must notify the tagging worker"
    );
}

#[test]
fn status_counts_include_tagging() {
    let mut ctx = DaemonAiTest::new();
    ctx.enable_ai(true, true);
    ctx.mark_plugin_ready();
    ctx.add_photo_and_scan();
    ctx.add_photo_and_scan();
    assert_eq!(ctx.tagging_counts().pending, 2);

    // A backlog job is unfinished too, even though the worker never claims
    // it until release.
    ctx.enable_ai(false, false);
    let (backlog_photo, _) = ctx.add_photo_and_scan();
    ctx.db
        .lock()
        .unwrap()
        .enqueue_tagging_jobs(&[backlog_photo], PLUGIN, TaggingJobStatus::Waiting)
        .unwrap();
    ctx.enable_ai(true, true);
    assert_eq!(ctx.tagging_counts().waiting, 1);
    assert_eq!(ctx.tagging_counts().unfinished(), 3);

    ctx.daemon.refresh_status().unwrap();
    {
        let mut status = ctx.daemon.status.lock().unwrap();
        assert_eq!(status.pending_tagging_jobs, 3);
        assert!(status.tagging_ready);
        let snapshot = status_snapshot(&mut status);
        assert_eq!(snapshot.pending_tagging_jobs, 3);
        assert!(snapshot.tagging_ready);
    }

    // A processing job still counts as unfinished.
    let claimed = {
        let db = ctx.db.lock().unwrap();
        db.next_pending_tagging_job().unwrap().unwrap()
    };
    assert_eq!(claimed.status, TaggingJobStatus::Processing);
    ctx.daemon.refresh_status().unwrap();
    assert_eq!(ctx.daemon.status.lock().unwrap().pending_tagging_jobs, 3);
}

#[test]
fn run_once_does_not_start_the_tagging_worker() {
    let mut ctx = DaemonAiTest::new();
    ctx.enable_ai(true, true);
    ctx.mark_plugin_ready();
    ctx.add_photo();

    ctx.daemon.run_once().unwrap();

    assert!(ctx.daemon.tagging_handle.is_none());
    assert_eq!(ctx.tagging_counts().pending, 1);
}

#[test]
fn start_workers_spawns_and_stops_the_tagging_worker() {
    let mut ctx = DaemonAiTest::new();
    ctx.daemon.start_workers();
    assert!(ctx.daemon.tagging_handle.is_some());
    ctx.daemon.stop_workers();
    assert!(ctx.daemon.tagging_handle.is_none());
}

#[test]
fn hash_worker_passes_missing_rows_and_hashes_later_photos() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);

    let valid_path = dir.path().join("photos/valid.bin");
    std::fs::write(&valid_path, b"hash me").unwrap();

    {
        let db = daemon.db.lock().unwrap();
        let root = db
            .add_root(dir.path().join("photos").to_str().unwrap(), "reference")
            .unwrap();
        for index in 0..16 {
            db.upsert_photo(&NewPhoto {
                root_id: root,
                rel_path: format!("missing-{index}.bin"),
                abs_path: dir
                    .path()
                    .join(format!("photos/missing-{index}.bin"))
                    .to_string_lossy()
                    .to_string(),
                file_name: format!("missing-{index}.bin"),
                captured_at_utc: index,
                captured_at_local: "2026-10-21 21:24:00.000".to_string(),
                tz_offset_sec: 9 * 3600,
                width: 1,
                height: 1,
                file_size: 1,
                mtime: index,
                blake3: None,
            })
            .unwrap();
        }
        db.upsert_photo(&NewPhoto {
            root_id: root,
            rel_path: "valid.bin".to_string(),
            abs_path: valid_path.to_string_lossy().to_string(),
            file_name: "valid.bin".to_string(),
            captured_at_utc: 100,
            captured_at_local: "2026-10-21 21:24:00.000".to_string(),
            tz_offset_sec: 9 * 3600,
            width: 1,
            height: 1,
            file_size: 7,
            mtime: 100,
            blake3: None,
        })
        .unwrap();
    }

    daemon.start_workers();
    wait_for(
        || {
            daemon
                .db
                .lock()
                .unwrap()
                .photo_by_path(valid_path.to_str().unwrap())
                .unwrap()
                .unwrap()
                .blake3
                .is_some()
        },
        Duration::from_secs(2),
    );
    daemon.stop_workers();
}

#[test]
fn startup_resumes_interrupted_tagging_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let photo_path = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&photo_path);
    let db_path = dir.path().join("library.db");
    let job_id = {
        let db = Db::open(&db_path).unwrap();
        let root = db
            .add_root(photo_dir.to_str().unwrap(), "reference")
            .unwrap();
        let photo_id = db
            .upsert_photo(&NewPhoto {
                root_id: root,
                rel_path: photo_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                abs_path: photo_path.to_string_lossy().into_owned(),
                file_name: photo_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                captured_at_utc: 1_700_000_000,
                captured_at_local: "2026-10-21 21:24:00".to_string(),
                tz_offset_sec: 9 * 3600,
                width: 16,
                height: 10,
                file_size: 10,
                mtime: 1_700_000_000,
                blake3: None,
            })
            .unwrap();
        db.enqueue_tagging_jobs(&[photo_id], PLUGIN, TaggingJobStatus::Pending)
            .unwrap();
        db.next_pending_tagging_job().unwrap().unwrap().id
    };

    // A maintenance process must not steal the live daemon's jobs.
    let preserving = Daemon::open_preserving_running_deletions(
        &db_path,
        Some(photo_dir.clone()),
        Some(log_dir.clone()),
    )
    .unwrap();
    assert_eq!(
        preserving
            .db
            .lock()
            .unwrap()
            .tagging_job(&job_id)
            .unwrap()
            .unwrap()
            .status,
        TaggingJobStatus::Processing
    );
    drop(preserving);

    let owning = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    assert_eq!(
        owning
            .db
            .lock()
            .unwrap()
            .tagging_job(&job_id)
            .unwrap()
            .unwrap()
            .status,
        TaggingJobStatus::Pending
    );
}

#[test]
fn force_delete_cancels_tagging_before_claim() {
    let mut ctx = DaemonAiTest::new();
    let (photo_id, path) = ctx.add_photo_and_scan();
    let job_id = {
        let db = ctx.db.lock().unwrap();
        db.enqueue_tagging_jobs(&[photo_id], PLUGIN, TaggingJobStatus::Pending)
            .unwrap();
        db.next_pending_tagging_job().unwrap().unwrap().id
    };
    let request = {
        let db = ctx.db.lock().unwrap();
        db.request_deletion(&[photo_id], DeletionReason::Manual, true)
            .unwrap()[0]
    };

    ctx.daemon.process_deletions().unwrap();

    let db = ctx.db.lock().unwrap();
    assert_eq!(
        db.tagging_job(&job_id).unwrap().unwrap().status,
        TaggingJobStatus::Cancelled
    );
    assert_eq!(
        db.deletion_request_by_id(request).unwrap().unwrap().status,
        DeletionStatus::Completed
    );
    assert!(!path.exists(), "the forced deletion must remove the file");
}

#[test]
fn forced_deletion_batch_cancels_all_tagging_in_one_pass() {
    let mut ctx = DaemonAiTest::new();
    let (photos, requests) = {
        let db = ctx.db.lock().unwrap();
        let root = db
            .add_root(ctx.photo_dir.to_str().unwrap(), "reference")
            .unwrap();
        let mut photos = Vec::new();
        let mut requests = Vec::new();
        for index in 0..150i64 {
            let name = format!("bulk{index:03}.png");
            let abs_path = ctx.photo_dir.join(&name);
            let photo_id = db
                .upsert_photo(&NewPhoto {
                    root_id: root,
                    rel_path: name.clone(),
                    abs_path: abs_path.to_string_lossy().into_owned(),
                    file_name: name,
                    captured_at_utc: 1_700_000_000 + index,
                    captured_at_local: "2026-10-21 21:24:00".to_string(),
                    tz_offset_sec: 9 * 3600,
                    width: 16,
                    height: 10,
                    file_size: 10,
                    mtime: 1_700_000_000 + index,
                    blake3: None,
                })
                .unwrap();
            db.enqueue_tagging_jobs(&[photo_id], PLUGIN, TaggingJobStatus::Pending)
                .unwrap();
            let request = db
                .request_deletion(&[photo_id], DeletionReason::Manual, true)
                .unwrap()[0];
            photos.push(photo_id);
            requests.push(request);
        }
        (photos, requests)
    };
    assert_eq!(ctx.tagging_counts().unfinished(), 150);

    ctx.daemon.process_deletions().unwrap();

    // One bulk statement cancels every job, even though the executor only
    // claims 100 requests per pass.
    assert_eq!(ctx.tagging_counts().unfinished(), 0);
    {
        let db = ctx.db.lock().unwrap();
        let completed = requests
            .iter()
            .filter(|request| {
                db.deletion_request_by_id(**request)
                    .unwrap()
                    .unwrap()
                    .status
                    == DeletionStatus::Completed
            })
            .count();
        assert_eq!(completed, 100);
        for photo_id in &photos {
            let jobs = db.tagging_jobs_for_photo(*photo_id).unwrap();
            assert!(
                jobs.iter()
                    .all(|job| job.status == TaggingJobStatus::Cancelled)
            );
        }
    }

    ctx.daemon.process_deletions().unwrap();
    let db = ctx.db.lock().unwrap();
    for request in requests {
        assert_eq!(
            db.deletion_request_by_id(request).unwrap().unwrap().status,
            DeletionStatus::Completed
        );
    }
}

#[test]
fn deletion_waits_for_processing_tagging() {
    let mut ctx = DaemonAiTest::new();
    let (photo_id, path) = ctx.add_photo_and_scan();
    let (job_id, request) = {
        let db = ctx.db.lock().unwrap();
        db.enqueue_tagging_jobs(&[photo_id], PLUGIN, TaggingJobStatus::Pending)
            .unwrap();
        let job_id = db.next_pending_tagging_job().unwrap().unwrap().id;
        let request = db
            .request_deletion(&[photo_id], DeletionReason::Manual, false)
            .unwrap()[0];
        (job_id, request)
    };

    ctx.daemon.process_deletions().unwrap();
    {
        let db = ctx.db.lock().unwrap();
        assert_eq!(
            db.tagging_job(&job_id).unwrap().unwrap().status,
            TaggingJobStatus::Processing,
            "a non-forced deletion must not cancel the job"
        );
        assert_eq!(
            db.deletion_request_by_id(request).unwrap().unwrap().status,
            DeletionStatus::Pending
        );
    }
    assert!(path.exists(), "the photo must survive while tagging runs");

    ctx.db.lock().unwrap().cancel_tagging_job(&job_id).unwrap();
    ctx.daemon.process_deletions().unwrap();

    let db = ctx.db.lock().unwrap();
    assert_eq!(
        db.deletion_request_by_id(request).unwrap().unwrap().status,
        DeletionStatus::Completed
    );
    assert!(!path.exists());
}

fn original_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path()
        .join("photos/VRChat_2026-10-21_21-24-00.000_4x3.png")
}

fn replace_test_daemon(dir: &tempfile::TempDir) -> Daemon {
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    write_photo(&original_path(dir));
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.set_ipc_endpoint(None);
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    save_daemon_settings(&daemon, |settings| {
        settings.auto_compress_backlog = true;
        settings.auto_compress_keep_original = false;
        settings.auto_compress_immediate = true;
    });
    daemon
}

fn manual_test_daemon(dir: &tempfile::TempDir) -> Daemon {
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    write_photo(&original_path(dir));
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.set_ipc_endpoint(None);
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    daemon
}

fn multi_profile_test_daemon(dir: &tempfile::TempDir) -> Daemon {
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.set_ipc_endpoint(None);
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    daemon
}

fn write_gradient_photo(path: &Path) {
    let image = image::RgbaImage::from_fn(64, 64, |x, y| {
        image::Rgba([(x * 4) as u8, (y * 4) as u8, ((x + y) * 2) as u8, 255])
    });
    image.save(path).unwrap();
}

fn save_batch_profile(
    db: &Db,
    id: &str,
    format: vrc_photon_core::model::CompressionFormat,
    suffix: &str,
    resize: Option<(u32, u32)>,
) {
    db.save_profile(&CompressionProfile {
        id: id.to_string(),
        name: id.to_string(),
        format,
        suffix: suffix.to_string(),
        resize_enabled: resize.is_some(),
        width: resize.map(|(w, _)| w).unwrap_or(3840),
        height: resize.map(|(_, h)| h).unwrap_or(2160),
        ..CompressionProfile::default()
    })
    .unwrap();
}

fn wait_until_idle(db: &Arc<Mutex<Db>>, timeout: Duration) {
    let started = Instant::now();
    loop {
        let (active, waiting) = {
            let db = db.lock().unwrap();
            (
                db.count_active_jobs().unwrap(),
                db.count_jobs(QueueStatus::Waiting).unwrap(),
            )
        };
        if active == 0 && waiting == 0 {
            return;
        }
        assert!(
            started.elapsed() < timeout,
            "jobs did not finish within {timeout:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for(mut condition: impl FnMut() -> bool, timeout: Duration) {
    let started = Instant::now();
    while !condition() {
        assert!(
            started.elapsed() < timeout,
            "condition was not met within {timeout:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn write_thumbnail_cache_entry(path: &Path, bytes: usize, modified: SystemTime) {
    std::fs::write(path, vec![0u8; bytes]).unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
}

#[test]
fn thumbnail_spec_pack_round_trips_the_width() {
    for width in [256, 384, 512] {
        let spec = thumbnail::ThumbnailSpec::new(width);
        assert_eq!(unpack_thumbnail_spec(pack_thumbnail_spec(spec)), spec);
    }
}

#[test]
fn refresh_compression_settings_clears_the_cache_on_a_size_change() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let cache = dir.path().join("thumbnails");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("1.bc7"), vec![0u8; 384 * 216]).unwrap();
    daemon.set_thumbnail_dir(cache.clone());
    let epoch = daemon.thumbnail_cache_epoch.load(Ordering::Relaxed);

    save_daemon_settings(&daemon, |settings| settings.thumbnail_size = 512);
    daemon.refresh_compression_settings();

    assert!(!cache.join("1.bc7").exists());
    assert!(daemon.thumbnail_cache_epoch.load(Ordering::Relaxed) > epoch);
    assert_eq!(daemon.settings.thumbnail_size, 512);
}

#[test]
fn startup_prunes_thumbnail_cache_to_limit() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let cache = dir.path().join("thumbnails");
    std::fs::create_dir_all(&cache).unwrap();
    let now = SystemTime::now();
    for (id, age) in [(1, 30), (2, 20), (3, 10)] {
        write_thumbnail_cache_entry(
            &cache.join(format!("{id}.bc7")),
            600_000,
            now - Duration::from_secs(age),
        );
    }
    save_daemon_settings(&daemon, |settings| settings.thumbnail_cache_mb = 1);
    daemon.refresh_compression_settings();
    daemon.set_thumbnail_dir(cache.clone());

    daemon.start_workers();
    wait_for(
        || !cache.join("1.bc7").exists() && !cache.join("2.bc7").exists(),
        Duration::from_secs(5),
    );
    assert!(cache.join("3.bc7").exists());
    daemon.stop_workers();
}

#[test]
fn run_once_prunes_thumbnail_cache_after_direct_generation() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let cache = dir.path().join("thumbnails");
    std::fs::create_dir_all(&cache).unwrap();
    let now = SystemTime::now();
    for (id, age) in [(1, 30), (2, 20), (3, 10)] {
        write_thumbnail_cache_entry(
            &cache.join(format!("{id}.bc7")),
            600_000,
            now - Duration::from_secs(age),
        );
    }
    save_daemon_settings(&daemon, |settings| settings.thumbnail_cache_mb = 1);
    daemon.refresh_compression_settings();
    daemon.set_thumbnail_dir(cache.clone());
    write_photo(
        &dir.path()
            .join("photos/VRChat_2026-10-21_21-24-00.000_4x3.png"),
    );

    daemon.run_once().unwrap();

    assert!(!cache.join("1.bc7").exists());
    assert!(!cache.join("2.bc7").exists());
    assert!(cache.join("3.bc7").exists());
}

fn report_with_remaining(bytes: u64) -> thumbnail::ThumbnailPruneReport {
    thumbnail::ThumbnailPruneReport {
        remaining_bytes: bytes,
        ..thumbnail::ThumbnailPruneReport::default()
    }
}

#[test]
fn thumbnail_budget_prunes_only_above_the_limit() {
    let mut budget = ThumbnailCacheBudget::new(1000);
    assert!(budget.needs_prune()); // 未計測
    budget.note_pruned(&report_with_remaining(1000));
    assert!(!budget.needs_prune());
    budget.note_generated(1);
    assert!(budget.needs_prune());
}

#[test]
fn thumbnail_budget_tracks_generation_external_growth_and_reset() {
    let mut budget = ThumbnailCacheBudget::new(1000);
    budget.note_pruned(&report_with_remaining(500));
    budget.note_generated(200);
    budget.note_external(100);
    assert!(!budget.needs_prune());
    budget.note_generated(300);
    assert!(budget.needs_prune());
    budget.note_pruned(&report_with_remaining(400));
    assert!(!budget.needs_prune());
}

#[test]
fn thumbnail_budget_never_prunes_when_unlimited() {
    let mut budget = ThumbnailCacheBudget::new(0);
    budget.note_generated(10_000_000);
    budget.note_external(10_000_000);
    assert!(!budget.needs_prune());
    budget.note_pruned(&report_with_remaining(0));
    assert!(!budget.needs_prune());
}

#[test]
fn external_thumbnail_growth_prunes_over_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let cache = dir.path().join("thumbnails");
    std::fs::create_dir_all(&cache).unwrap();
    let now = SystemTime::now();
    for (id, age) in [(1, 30), (2, 20), (3, 10)] {
        write_thumbnail_cache_entry(
            &cache.join(format!("{id}.bc7")),
            600_000,
            now - Duration::from_secs(age),
        );
    }
    save_daemon_settings(&daemon, |settings| settings.thumbnail_cache_mb = 2);
    daemon.refresh_compression_settings();
    daemon.set_thumbnail_dir(cache.clone());

    daemon.start_workers();
    // The startup pass discards notifications, so consume two pulses: the
    // second one can only be handled by the idle branch after startup.
    for _ in 0..2 {
        daemon
            .thumbnail_external_bytes
            .fetch_add(1, Ordering::Relaxed);
        wait_for(
            || daemon.thumbnail_external_bytes.load(Ordering::Relaxed) == 0,
            Duration::from_secs(5),
        );
    }
    assert!(cache.join("1.bc7").exists());

    // The UI writes its own thumbnails; the notice reports bytes the
    // daemon's generation counter never saw.
    write_thumbnail_cache_entry(&cache.join("4.bc7"), 1_000_000, now);
    daemon
        .thumbnail_external_bytes
        .fetch_add(1_000_000, Ordering::Relaxed);
    wait_for(|| !cache.join("1.bc7").exists(), Duration::from_secs(5));
    assert!(cache.join("3.bc7").exists());
    daemon.stop_workers();
}

#[test]
fn external_notice_prunes_through_a_channel_message() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let cache = dir.path().join("thumbnails");
    std::fs::create_dir_all(&cache).unwrap();
    let now = SystemTime::now();
    for (id, age) in [(1, 30), (2, 20), (3, 10)] {
        write_thumbnail_cache_entry(
            &cache.join(format!("{id}.bc7")),
            600_000,
            now - Duration::from_secs(age),
        );
    }
    save_daemon_settings(&daemon, |settings| settings.thumbnail_cache_mb = 2);
    daemon.refresh_compression_settings();
    daemon.set_thumbnail_dir(cache.clone());

    daemon.start_workers();
    for _ in 0..2 {
        daemon
            .thumbnail_external_bytes
            .fetch_add(1, Ordering::Relaxed);
        wait_for(
            || daemon.thumbnail_external_bytes.load(Ordering::Relaxed) == 0,
            Duration::from_secs(5),
        );
    }
    assert!(cache.join("1.bc7").exists());

    write_thumbnail_cache_entry(&cache.join("4.bc7"), 1_000_000, now);
    // Hold the DB lock so the worker must consume the notice in the Ok
    // branch and take the post-lookup prune path after we release it.
    let db = daemon.db.lock().unwrap();
    daemon
        .thumbnail_external_bytes
        .fetch_add(1_000_000, Ordering::Relaxed);
    daemon
        .thumbnail_tx
        .as_ref()
        .unwrap()
        .send(ThumbnailRequest::Photo(999_999))
        .unwrap();
    wait_for(
        || daemon.thumbnail_external_bytes.load(Ordering::Relaxed) == 0,
        Duration::from_secs(5),
    );
    drop(db);

    wait_for(|| !cache.join("1.bc7").exists(), Duration::from_secs(5));
    assert!(cache.join("3.bc7").exists());
    daemon.stop_workers();
}

#[test]
fn thumbnail_budget_skips_a_prune_after_the_limit_turns_unlimited() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("thumbnails");
    std::fs::create_dir_all(&cache).unwrap();
    let path = cache.join("1.bc7");
    write_thumbnail_cache_entry(&path, 2_000, SystemTime::now());
    let cache_limit = AtomicU64::new(1000);
    let mut budget = ThumbnailCacheBudget::new(1000);
    budget.note_pruned(&report_with_remaining(2_000));
    assert!(budget.needs_prune());

    cache_limit.store(0, Ordering::Relaxed);
    refresh_thumbnail_budget_and_prune(&cache, &cache_limit, &mut budget);

    assert_eq!(budget.limit_bytes, 0);
    assert!(budget.known_bytes.is_none());
    assert!(!budget.needs_prune());
    assert!(path.exists());
}

#[test]
fn prune_thumbnail_cache_with_zero_limit_leaves_files_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("1.webp");
    write_thumbnail_cache_entry(&path, 2_000, SystemTime::now());

    assert!(prune_thumbnail_cache(dir.path(), 0).is_none());

    assert_eq!(std::fs::metadata(&path).unwrap().len(), 2_000);
}

#[test]
fn thumbnail_prune_schedule_fires_after_a_batch() {
    let mut schedule = ThumbnailPruneSchedule::new(2);

    assert!(!schedule.note_generated());
    assert!(schedule.note_generated());
    assert!(!schedule.has_pending());
}

#[test]
fn thumbnail_prune_schedule_resets_pending_after_an_idle_prune() {
    let mut schedule = ThumbnailPruneSchedule::new(128);

    assert!(!schedule.note_generated());
    assert!(schedule.has_pending());
    schedule.note_pruned();
    assert!(!schedule.has_pending());
}

#[test]
fn live_thumbnail_limit_change_prunes_cache() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let cache = dir.path().join("thumbnails");
    std::fs::create_dir_all(&cache).unwrap();
    let now = SystemTime::now();
    for (id, age) in [(1, 30), (2, 20), (3, 10)] {
        write_thumbnail_cache_entry(
            &cache.join(format!("{id}.bc7")),
            600_000,
            now - Duration::from_secs(age),
        );
    }
    daemon.set_thumbnail_dir(cache.clone());
    daemon.start_workers();
    assert!(cache.join("1.bc7").exists());

    save_daemon_settings(&daemon, |settings| settings.thumbnail_cache_mb = 1);
    daemon.refresh_compression_settings();
    wait_for(
        || !cache.join("1.bc7").exists() && !cache.join("2.bc7").exists(),
        Duration::from_secs(5),
    );
    assert!(cache.join("3.bc7").exists());
    daemon.stop_workers();
}

#[test]
fn replace_job_queues_deletion_instead_of_trashing_immediately() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = replace_test_daemon(&dir);
    daemon.run_once().unwrap();

    // Drive one job directly so the assertion sees the state between job
    // completion and deletion execution.
    let job = {
        let db = daemon.db.lock().unwrap();
        db.next_pending_job().unwrap().unwrap()
    };
    process_job(&daemon.db, &job).unwrap();

    let db = Db::open(dir.path().join("library.db")).unwrap();
    let photo = db
        .photo_by_path(original_path(&dir).to_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(photo.status, PhotoStatus::Replaced);
    assert!(
        original_path(&dir).exists(),
        "the original survives until the request runs"
    );
    let request = db.active_deletion_request(photo.id).unwrap().unwrap();
    assert_eq!(request.reason, DeletionReason::AutoReplace);
    assert_eq!(request.status, DeletionStatus::Pending);

    daemon.process_deletions().unwrap();
    assert!(!original_path(&dir).exists());
    assert_eq!(db.derivatives_for_photo(photo.id).unwrap().len(), 1);
    assert!(db.active_deletion_request(photo.id).unwrap().is_none());
}

#[test]
fn deletion_waits_for_every_job_of_the_photo() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = replace_test_daemon(&dir);
    daemon.run_once().unwrap();
    let db = Db::open(dir.path().join("library.db")).unwrap();
    let photo = db
        .photo_by_path(original_path(&dir).to_str().unwrap())
        .unwrap()
        .unwrap();
    db.save_profile(&CompressionProfile {
        id: "second".to_string(),
        name: "Second".to_string(),
        ..CompressionProfile::default()
    })
    .unwrap();
    db.enqueue_job(photo.id, "second", "keep").unwrap();
    db.request_deletion(&[photo.id], DeletionReason::Manual, false)
        .unwrap();

    daemon.process_deletions().unwrap();
    assert!(
        original_path(&dir).exists(),
        "pending job must block deletion"
    );
    daemon.compress_pending().unwrap();
    daemon.process_deletions().unwrap();
    assert!(!original_path(&dir).exists());
    assert_eq!(
        db.photo_by_id(photo.id).unwrap().unwrap().status,
        PhotoStatus::Missing
    );
}

#[test]
fn manual_deletion_removes_derivatives_and_marks_photo_missing() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = replace_test_daemon(&dir);
    daemon.run_once().unwrap();
    daemon.compress_pending().unwrap();
    daemon.process_deletions().unwrap();

    let db = Db::open(dir.path().join("library.db")).unwrap();
    let photo = db
        .photo_by_path(original_path(&dir).to_str().unwrap())
        .unwrap()
        .unwrap();
    let derivative = db.derivatives_for_photo(photo.id).unwrap()[0].path.clone();
    db.request_deletion(&[photo.id], DeletionReason::Manual, false)
        .unwrap();
    daemon.process_deletions().unwrap();

    assert!(!original_path(&dir).exists());
    assert!(!Path::new(&derivative).exists());
    let photo = db.photo_by_id(photo.id).unwrap().unwrap();
    assert_eq!(photo.status, PhotoStatus::Missing);
    assert!(db.derivatives_for_photo(photo.id).unwrap().is_empty());
    assert_eq!(db.count_photos().unwrap(), 0);
}

#[test]
fn daemon_open_resumes_interrupted_compression_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = manual_test_daemon(&dir);
    daemon.run_once().unwrap();
    drop(daemon);

    let db_path = dir.path().join("library.db");
    let job_id = {
        let db = Db::open(&db_path).unwrap();
        let photo = db
            .photo_by_path(original_path(&dir).to_str().unwrap())
            .unwrap()
            .unwrap();
        let job = db.enqueue_job(photo.id, "p", "keep").unwrap();
        let claimed = db.next_pending_job().unwrap().unwrap();
        assert_eq!(claimed.id, job);
        job
    };

    let reopened = Daemon::open(
        &db_path,
        Some(dir.path().join("photos")),
        Some(dir.path().join("logs")),
    )
    .unwrap();
    drop(reopened);

    let db = Db::open(&db_path).unwrap();
    let resumed = db
        .list_jobs(10)
        .unwrap()
        .into_iter()
        .find(|job| job.id == job_id)
        .expect("compression job");
    assert_eq!(resumed.status, QueueStatus::Pending);
    assert_eq!(resumed.retry_count, 0);
    assert!(resumed.started_at.is_none());
}

#[test]
fn startup_recovers_an_interrupted_compression_job_with_an_open_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = manual_test_daemon(&dir);
    daemon.run_once().unwrap();
    drop(daemon);

    let db_path = dir.path().join("library.db");
    let log_path = dir.path().join("logs/output_log_2026-10-21_21-00-00.txt");
    std::fs::write(&log_path, "").unwrap();
    let mtime: i64 = 1_700_000_500;
    let file = File::options().write(true).open(&log_path).unwrap();
    file.set_modified(std::time::UNIX_EPOCH + Duration::from_secs(mtime as u64))
        .unwrap();
    drop(file);

    let (job_id, session_id) = {
        let db = Db::open(&db_path).unwrap();
        let photo = db
            .photo_by_path(original_path(&dir).to_str().unwrap())
            .unwrap()
            .unwrap();
        let job_id = db.enqueue_job(photo.id, "p", "keep").unwrap();
        let claimed = db.next_pending_job().unwrap().unwrap();
        assert_eq!(claimed.id, job_id);
        assert_eq!(claimed.status, QueueStatus::Processing);

        let world = db
            .upsert_world("wrld_test", Some("Test World"), Some("log"))
            .unwrap();
        let session_id = db
            .create_session(log_path.to_str().unwrap(), mtime - 120, world, None)
            .unwrap();
        (job_id, session_id)
    };

    let mut reopened = Daemon::open(
        &db_path,
        Some(dir.path().join("photos")),
        Some(dir.path().join("logs")),
    )
    .unwrap();
    assert_eq!(
        reopened
            .repair_open_sessions_with_running(Some(false))
            .unwrap(),
        1
    );

    let db = Db::open(&db_path).unwrap();
    let resumed = db
        .list_jobs(10)
        .unwrap()
        .into_iter()
        .find(|job| job.id == job_id)
        .expect("compression job");
    assert_eq!(resumed.status, QueueStatus::Pending);
    assert_eq!(resumed.retry_count, 0);
    assert!(resumed.started_at.is_none());
    assert!(
        db.open_sessions().unwrap().is_empty(),
        "startup must close the stale session in the same pass"
    );
    let session = db.session_by_id(session_id).unwrap().unwrap();
    assert_eq!(session.ended_at, Some(mtime));
}

#[test]
fn concurrent_polling_log_ingestion_and_thumbnails_do_not_deadlock() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let photo_path = original_path(&dir);
    write_photo(&photo_path);
    let log_path = dir.path().join("logs/output_log_2026-10-21_21-00-00.txt");
    let join = "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)";
    let name = "2026.10.21 21:23:13 Debug      -  [Behaviour] Joining or Creating Room: Test World";
    std::fs::write(&log_path, format!("{join}\n{name}\n")).unwrap();
    daemon.run_once().unwrap();

    let db_path = dir.path().join("library.db");
    let (photo_id, thumbnail_spec) = {
        let db = daemon.db.lock().unwrap();
        let photo = db
            .photo_by_path(photo_path.to_str().unwrap())
            .unwrap()
            .unwrap();
        (photo.id, daemon.settings.thumbnail_spec())
    };
    let cache = dir.path().join("thumbnails");
    std::fs::create_dir_all(&cache).unwrap();
    let db = Arc::clone(&daemon.db);
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let (tx, rx) = std::sync::mpsc::channel();

    let poller_db = Arc::clone(&db);
    let poller_barrier = Arc::clone(&barrier);
    let poller_tx = tx.clone();
    let poller = thread::spawn(move || {
        poller_barrier.wait();
        let result = (|| -> std::result::Result<(), String> {
            for _ in 0..200 {
                let db = poller_db.lock().map_err(|error| error.to_string())?;
                db.stats().map_err(|error| error.to_string())?;
                db.list_photos(10, 0, vrc_photon_core::model::PhotoSort::Newest)
                    .map_err(|error| error.to_string())?;
                db.library_revision().map_err(|error| error.to_string())?;
                drop(db);
                thread::yield_now();
            }
            Ok(())
        })();
        let _ = poller_tx.send(result);
    });

    let thumbnail_db = Arc::clone(&db);
    let thumbnail_barrier = Arc::clone(&barrier);
    let thumbnail_tx = tx.clone();
    let thumbnail_cache = cache.clone();
    let thumbnail_source = photo_path.clone();
    let thumbnail = thread::spawn(move || {
        thumbnail_barrier.wait();
        let result = (|| -> std::result::Result<(), String> {
            for _ in 0..16 {
                let source = {
                    let db = thumbnail_db.lock().map_err(|error| error.to_string())?;
                    let photo = db
                        .photo_by_id(photo_id)
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| "photo disappeared".to_string())?;
                    PathBuf::from(photo.effective_path())
                };
                assert_eq!(source, thumbnail_source);
                let thumbnail = thumbnail::ensure_thumbnail_in(
                    photo_id,
                    &source,
                    &thumbnail_cache,
                    thumbnail_spec,
                )
                .map_err(|error| error.to_string())?;
                if thumbnail.is_none() {
                    return Err("thumbnail was not generated".to_string());
                }
            }
            Ok(())
        })();
        let _ = thumbnail_tx.send(result);
    });

    barrier.wait();
    let mut file = File::options().append(true).open(&log_path).unwrap();
    for index in 0..16 {
        writeln!(
            file,
            "2026.10.21 21:24:{index:02} Debug      -  [Behaviour] OnPlayerJoined Player {index} (usr_{index})"
        )
        .unwrap();
        if index % 4 == 0 {
            daemon.ingest_logs().unwrap();
        }
    }
    daemon.ingest_logs().unwrap();

    for _ in 0..2 {
        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(result) => result.unwrap(),
            Err(error) => panic!("concurrent worker did not finish: {error}"),
        }
    }
    poller.join().unwrap();
    thumbnail.join().unwrap();

    // The shared database must still be usable after all three paths ran.
    let db = Db::open(&db_path).unwrap();
    assert_eq!(db.count_photos().unwrap(), 1);
    assert_eq!(db.stats().unwrap().photo_count, 1);
    assert_eq!(
        db.list_photos(10, 0, vrc_photon_core::model::PhotoSort::Newest)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(db.open_sessions().unwrap().len(), 1);
    let thumbnail = thumbnail::thumbnail_path_in(&cache, photo_id, thumbnail_spec);
    assert!(thumbnail.is_file(), "thumbnail must survive the race");
}

#[test]
fn maintenance_open_preserves_a_live_daemons_running_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = manual_test_daemon(&dir);
    daemon.run_once().unwrap();
    let db_path = dir.path().join("library.db");
    let photo = {
        let db = Db::open(&db_path).unwrap();
        let photo = db
            .photo_by_path(original_path(&dir).to_str().unwrap())
            .unwrap()
            .unwrap();
        let request = db
            .request_deletion(&[photo.id], DeletionReason::Manual, false)
            .unwrap()[0];
        assert!(db.claim_deletion(request).unwrap().is_some());
        photo
    };

    // A maintenance command opens the same database while the first
    // daemon still owns the running request.
    let mut maintenance = Daemon::open_preserving_running_deletions(
        &db_path,
        Some(dir.path().join("photos")),
        Some(dir.path().join("logs")),
    )
    .unwrap();
    maintenance.run_once().unwrap();
    {
        let db = Db::open(&db_path).unwrap();
        let request = db.active_deletion_request(photo.id).unwrap().unwrap();
        assert_eq!(request.status, DeletionStatus::Running);
    }
    drop(maintenance);
    drop(daemon);

    // Once no executor remains, the next normal startup recovers it.
    let _recovered = Daemon::open(
        &db_path,
        Some(dir.path().join("photos")),
        Some(dir.path().join("logs")),
    )
    .unwrap();
    let db = Db::open(&db_path).unwrap();
    let request = db.active_deletion_request(photo.id).unwrap().unwrap();
    assert_eq!(request.status, DeletionStatus::Pending);
}

#[test]
fn preserving_open_leaves_pending_deletions_to_the_executor() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = manual_test_daemon(&dir);
    daemon.run_once().unwrap();
    let db_path = dir.path().join("library.db");
    let photo = {
        let db = Db::open(&db_path).unwrap();
        let photo = db
            .photo_by_path(original_path(&dir).to_str().unwrap())
            .unwrap()
            .unwrap();
        db.request_deletion(&[photo.id], DeletionReason::Manual, false)
            .unwrap();
        photo
    };

    let mut maintenance = Daemon::open_preserving_running_deletions(
        &db_path,
        Some(dir.path().join("photos")),
        Some(dir.path().join("logs")),
    )
    .unwrap();
    maintenance.process_deletions().unwrap();
    {
        let db = Db::open(&db_path).unwrap();
        let request = db.active_deletion_request(photo.id).unwrap().unwrap();
        assert_eq!(request.status, DeletionStatus::Pending);
    }
    assert!(original_path(&dir).exists());

    drop(maintenance);
    drop(daemon);

    let mut owner = Daemon::open(
        &db_path,
        Some(dir.path().join("photos")),
        Some(dir.path().join("logs")),
    )
    .unwrap();
    owner.process_deletions().unwrap();
    assert!(!original_path(&dir).exists());
    let db = Db::open(&db_path).unwrap();
    assert!(db.active_deletion_request(photo.id).unwrap().is_none());
}

#[test]
fn deletion_recovers_after_partial_file_removal() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = manual_test_daemon(&dir);
    daemon.run_once().unwrap();
    let db_path = dir.path().join("library.db");
    let photo = {
        let db = Db::open(&db_path).unwrap();
        let photo = db
            .photo_by_path(original_path(&dir).to_str().unwrap())
            .unwrap()
            .unwrap();
        let request = db
            .request_deletion(&[photo.id], DeletionReason::Manual, false)
            .unwrap()[0];
        assert!(db.claim_deletion(request).unwrap().is_some());
        photo
    };
    std::fs::remove_file(original_path(&dir)).unwrap();
    drop(daemon);

    // Reopen through the production path: the running request must return
    // to pending while keeping the partial-execution marker.
    let mut daemon = Daemon::open(
        &db_path,
        Some(dir.path().join("photos")),
        Some(dir.path().join("logs")),
    )
    .unwrap();
    daemon.set_ipc_endpoint(None);
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    {
        let db = Db::open(&db_path).unwrap();
        let request = db.active_deletion_request(photo.id).unwrap().unwrap();
        assert_eq!(request.status, DeletionStatus::Pending);
        assert!(request.started_at.is_some());
        assert!(!db.cancel_deletion(request.id).unwrap());
    }

    daemon.process_deletions().unwrap();
    let db = Db::open(&db_path).unwrap();
    assert!(db.active_deletion_request(photo.id).unwrap().is_none());
    assert_eq!(
        db.photo_by_id(photo.id).unwrap().unwrap().status,
        PhotoStatus::Missing
    );
}

#[test]
fn cancel_blocked_job_failed_manual_deletion_keeps_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = manual_test_daemon(&dir);
    daemon.run_once().unwrap();
    let db_path = dir.path().join("library.db");
    let db = Db::open(&db_path).unwrap();
    let photo = db
        .photo_by_path(original_path(&dir).to_str().unwrap())
        .unwrap()
        .unwrap();
    let derivative_path = dir.path().join("photos/kept.webp");
    std::fs::copy(original_path(&dir), &derivative_path).unwrap();
    db.add_derivative(
        photo.id,
        derivative_path.to_str().unwrap(),
        "webp",
        16,
        10,
        std::fs::metadata(&derivative_path).unwrap().len(),
        Some("p"),
        CompressionOrigin::Manual,
        "keep",
    )
    .unwrap();
    db.save_profile(&CompressionProfile {
        id: "p".to_string(),
        name: "P".to_string(),
        ..CompressionProfile::default()
    })
    .unwrap();
    let failed = db.enqueue_job(photo.id, "p", "keep").unwrap();
    db.update_job(&failed, QueueStatus::Failed, 0.0, Some("boom"), None)
        .unwrap();
    let request = db
        .request_deletion(&[photo.id], DeletionReason::Manual, false)
        .unwrap()[0];
    daemon.process_deletions().unwrap();
    assert_eq!(
        db.active_deletion_request(photo.id)
            .unwrap()
            .unwrap()
            .status,
        DeletionStatus::Blocked
    );

    assert!(db.cancel_deletion(request).unwrap());
    assert!(original_path(&dir).exists());
    assert!(derivative_path.exists());
    assert_eq!(
        db.photo_by_id(photo.id).unwrap().unwrap().status,
        PhotoStatus::Present
    );
    assert!(db.active_deletion_request(photo.id).unwrap().is_none());
}

#[test]
fn auto_replace_without_a_derivative_keeps_the_original() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = replace_test_daemon(&dir);
    daemon.run_once().unwrap();
    let job = {
        let db = daemon.db.lock().unwrap();
        db.next_pending_job().unwrap().unwrap()
    };
    process_job(&daemon.db, &job).unwrap();
    let db = Db::open(dir.path().join("library.db")).unwrap();
    let photo = db
        .photo_by_path(original_path(&dir).to_str().unwrap())
        .unwrap()
        .unwrap();
    let derivative = db.derivatives_for_photo(photo.id).unwrap()[0].path.clone();
    std::fs::remove_file(&derivative).unwrap();

    daemon.process_deletions().unwrap();
    assert!(original_path(&dir).exists(), "the only copy must survive");
    let request = db.active_deletion_request(photo.id).unwrap().unwrap();
    assert_eq!(request.status, DeletionStatus::Blocked);
    assert_eq!(request.blocked_reason.as_deref(), Some("missing_asset"));
}

#[test]
fn preferred_display_derivative_uses_only_auto_origin_then_newest_id() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.webp");
    let second = dir.path().join("second.webp");
    let missing = dir.path().join("missing.webp");
    std::fs::write(&first, b"a").unwrap();
    std::fs::write(&second, b"b").unwrap();
    let derivative = |id: i64, origin: CompressionOrigin, profile: Option<&str>, path: &Path| {
        vrc_photon_core::model::Derivative {
            id,
            photo_id: 1,
            path: path.to_string_lossy().to_string(),
            format: "webp".to_string(),
            width: 16,
            height: 10,
            file_size: 1,
            profile_id: profile.map(str::to_string),
            origin,
            policy: "replace".to_string(),
            created_at: String::new(),
        }
    };
    let derivatives = vec![
        derivative(2, CompressionOrigin::Auto, None, &second),
        derivative(1, CompressionOrigin::Auto, None, &first),
        // Manual derivatives never become the display file.
        derivative(9, CompressionOrigin::Manual, Some("profile"), &first),
    ];

    let picked = preferred_display_derivative(&derivatives).unwrap();
    assert_eq!(picked.id, 2);

    // Missing files are not candidates.
    let absent = vec![derivative(5, CompressionOrigin::Auto, None, &missing)];
    assert!(preferred_display_derivative(&absent).is_none());

    // Manual-only output is not a display derivative.
    let manual = vec![derivative(
        7,
        CompressionOrigin::Manual,
        Some("profile"),
        &first,
    )];
    assert!(preferred_display_derivative(&manual).is_none());
}

#[test]
fn ready_deletion_after_more_than_hundred_blocked_requests_runs() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let db = Db::open(dir.path().join("library.db")).unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let mut request_ids = Vec::new();

    for index in 0..=100i64 {
        let name = format!("p{index:03}.png");
        let photo_id = db
            .upsert_photo(&vrc_photon_core::model::NewPhoto {
                root_id: root,
                rel_path: name.clone(),
                abs_path: format!("/photos/{name}"),
                file_name: name,
                captured_at_utc: 1_700_000_000 + index,
                captured_at_local: "2026-10-21 21:14:03".to_string(),
                tz_offset_sec: 9 * 3600,
                width: 16,
                height: 10,
                file_size: 10,
                mtime: 1_700_000_000 + index,
                blake3: None,
            })
            .unwrap();
        if index < 100 {
            db.enqueue_job(photo_id, "missing-profile", "keep").unwrap();
        }
        let request = db
            .request_deletion(&[photo_id], DeletionReason::Manual, false)
            .unwrap()[0];
        request_ids.push(request);
    }

    daemon.process_deletions().unwrap();

    let ready = db
        .deletion_request_by_id(request_ids[100])
        .unwrap()
        .unwrap();
    assert_eq!(ready.status, DeletionStatus::Completed);
    for request_id in &request_ids[..100] {
        let pending = db.deletion_request_by_id(*request_id).unwrap().unwrap();
        assert_eq!(pending.status, DeletionStatus::Pending);
    }
}

#[test]
fn manual_deletion_preempts_an_older_auto_replace_backlog() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let db = Db::open(dir.path().join("library.db")).unwrap();
    let root = db.add_root("/photos", "reference").unwrap();

    for index in 0..100i64 {
        let name = format!("auto{index:03}.png");
        let photo_id = db
            .upsert_photo(&vrc_photon_core::model::NewPhoto {
                root_id: root,
                rel_path: name.clone(),
                abs_path: format!("/photos/{name}"),
                file_name: name,
                captured_at_utc: 1_700_000_000 + index,
                captured_at_local: "2026-10-21 21:14:03".to_string(),
                tz_offset_sec: 9 * 3600,
                width: 16,
                height: 10,
                file_size: 10,
                mtime: 1_700_000_000 + index,
                blake3: None,
            })
            .unwrap();
        let request = db
            .request_deletion(&[photo_id], DeletionReason::AutoReplace, false)
            .unwrap()[0];
        db.connection()
            .execute(
                "UPDATE deletion_requests SET requested_at = ?1 WHERE id = ?2",
                ("2026-10-21T00:00:01Z", request),
            )
            .unwrap();
    }

    let manual_path = dir.path().join("photos/manual.png");
    write_photo(&manual_path);
    let manual_id = db
        .upsert_photo(&vrc_photon_core::model::NewPhoto {
            root_id: root,
            rel_path: "manual.png".to_string(),
            abs_path: manual_path.to_string_lossy().to_string(),
            file_name: "manual.png".to_string(),
            captured_at_utc: 1_700_000_500,
            captured_at_local: "2026-10-21 21:14:03".to_string(),
            tz_offset_sec: 9 * 3600,
            width: 16,
            height: 10,
            file_size: 10,
            mtime: 1_700_000_500,
            blake3: None,
        })
        .unwrap();
    let manual_request = db
        .request_deletion(&[manual_id], DeletionReason::Manual, false)
        .unwrap()[0];
    db.connection()
        .execute(
            "UPDATE deletion_requests SET requested_at = ?1 WHERE id = ?2",
            ("2026-10-21T00:00:02Z", manual_request),
        )
        .unwrap();

    daemon.process_deletions().unwrap();

    let request = db.deletion_request_by_id(manual_request).unwrap().unwrap();
    assert_eq!(
        request.status,
        DeletionStatus::Completed,
        "the manual deletion must not wait behind the auto_replace backlog"
    );
    assert!(
        !manual_path.exists(),
        "the manual deletion must remove the file"
    );
}

#[test]
fn unknown_deletion_reason_is_blocked_without_deleting() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let original = dir
        .path()
        .join("photos/VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&original);
    daemon.run_once().unwrap();

    let db = Db::open(dir.path().join("library.db")).unwrap();
    let photo = db
        .photo_by_path(original.to_str().unwrap())
        .unwrap()
        .unwrap();
    let derivative = dir.path().join("photos/kept.webp");
    std::fs::write(&derivative, b"kept derivative").unwrap();
    db.add_derivative(
        photo.id,
        derivative.to_str().unwrap(),
        "webp",
        16,
        10,
        std::fs::metadata(&derivative).unwrap().len(),
        Some("p"),
        CompressionOrigin::Manual,
        "keep",
    )
    .unwrap();
    db.connection()
        .execute(
            "INSERT INTO deletion_requests(photo_id, reason, status, force, requested_at) \
             VALUES (?1, 'future_reason', 'pending', 0, ?2)",
            (photo.id, chrono::Utc::now().to_rfc3339()),
        )
        .unwrap();

    daemon.process_deletions().unwrap();

    assert!(original.exists(), "unknown reason must not delete files");
    assert!(
        derivative.exists(),
        "unknown reason must not delete derivatives"
    );
    assert_eq!(
        db.derivatives_for_photo(photo.id).unwrap().len(),
        1,
        "unknown reason must not drop the derivative row"
    );
    let request = db.active_deletion_request(photo.id).unwrap().unwrap();
    assert_eq!(
        request.reason,
        vrc_photon_core::model::DeletionReason::Unknown
    );
    assert_eq!(request.status, DeletionStatus::Blocked);
    assert_eq!(request.blocked_reason.as_deref(), Some("unknown_reason"));
}

#[test]
fn unknown_reason_upgraded_to_manual_is_executed_not_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let original = dir
        .path()
        .join("photos/VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&original);
    daemon.run_once().unwrap();

    let db = Db::open(dir.path().join("library.db")).unwrap();
    let photo = db
        .photo_by_path(original.to_str().unwrap())
        .unwrap()
        .unwrap();
    db.connection()
        .execute(
            "INSERT INTO deletion_requests(photo_id, reason, status, force, requested_at) \
             VALUES (?1, 'future_reason', 'pending', 0, ?2)",
            (photo.id, chrono::Utc::now().to_rfc3339()),
        )
        .unwrap();
    // The UI upgrades the request to manual before the executor acts on
    // its snapshot of the unknown reason.
    let request_id = db
        .request_deletion(
            &[photo.id],
            vrc_photon_core::model::DeletionReason::Manual,
            false,
        )
        .unwrap()[0];

    daemon.process_deletions().unwrap();

    assert!(
        !original.exists(),
        "the upgraded manual deletion must remove the file"
    );
    let request = db.deletion_request_by_id(request_id).unwrap().unwrap();
    assert_eq!(
        request.reason,
        vrc_photon_core::model::DeletionReason::Manual
    );
    assert_eq!(request.status, DeletionStatus::Completed);
    assert!(request.blocked_reason.is_none());
}

#[test]
fn auto_replace_keeps_the_auto_derivative_as_the_display_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = replace_test_daemon(&dir);
    let db_path = dir.path().join("library.db");
    let original = original_path(&dir);
    daemon.run_once().unwrap();
    daemon.compress_pending().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db
        .photo_by_path(original.to_str().unwrap())
        .unwrap()
        .unwrap();
    let auto = db
        .derivatives_for_photo(photo.id)
        .unwrap()
        .into_iter()
        .find(|derivative| derivative.origin == CompressionOrigin::Auto)
        .expect("auto derivative");
    assert_eq!(photo.status, PhotoStatus::Replaced);
    assert_eq!(photo.display_derivative_id, Some(auto.id));

    let manual_path = dir.path().join("manual.webp");
    std::fs::write(&manual_path, b"manual").unwrap();
    let manual = db
        .add_derivative(
            photo.id,
            manual_path.to_str().unwrap(),
            "webp",
            16,
            10,
            5,
            Some("lossy"),
            CompressionOrigin::Manual,
            "replace",
        )
        .unwrap();
    assert_ne!(manual, auto.id);
    db.set_replaced(photo.id, manual).unwrap();
    db.request_deletion(&[photo.id], DeletionReason::AutoReplace, false)
        .unwrap();

    daemon.process_deletions().unwrap();

    let photo = db.photo_by_id(photo.id).unwrap().unwrap();
    assert_eq!(photo.display_derivative_id, Some(auto.id));
    assert_ne!(photo.display_derivative_id, Some(manual));
    assert!(manual_path.is_file());
}

#[test]
fn auto_replace_is_worker_count_independent() {
    for workers in 1usize..=4 {
        let dir = tempfile::tempdir().unwrap();
        let mut daemon = multi_profile_test_daemon(&dir);
        let db_path = dir.path().join("library.db");
        save_daemon_settings(&daemon, |settings| {
            settings.auto_compress_backlog = true;
            settings.auto_compress_keep_original = false;
            settings.auto_compress_immediate = true;
        });
        let original = original_path(&dir);
        write_gradient_photo(&original);
        let expected = vrc_photon_core::image_decode::decode_image(&original)
            .unwrap()
            .to_rgba8();
        daemon.run_once().unwrap();
        daemon.settings.compression_workers = workers;
        daemon.start_workers();
        wait_until_idle(&daemon.db, Duration::from_secs(15));
        daemon.stop_workers();
        daemon.process_deletions().unwrap();

        let db = Db::open(&db_path).unwrap();
        let photo = db
            .photo_by_path(original.to_str().unwrap())
            .unwrap()
            .unwrap();
        let derivatives = db.derivatives_for_photo(photo.id).unwrap();
        assert_eq!(derivatives.len(), 1, "workers={workers}");
        assert_eq!(
            derivatives[0].origin,
            CompressionOrigin::Auto,
            "workers={workers}"
        );
        assert!(Path::new(&derivatives[0].path).is_file());
        assert!(!original.exists(), "workers={workers}");
        assert_eq!(photo.status, PhotoStatus::Replaced);
        assert_eq!(db.count_jobs(QueueStatus::Completed).unwrap(), 1);
        assert_eq!(db.count_jobs(QueueStatus::Failed).unwrap(), 0);
        let completed: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM deletion_requests WHERE status = 'completed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(completed, 1, "workers={workers}");

        let display_id = photo.display_derivative_id.expect("display derivative");
        assert_eq!(display_id, derivatives[0].id);
        let actual = vrc_photon_core::image_decode::decode_image(Path::new(&derivatives[0].path))
            .unwrap()
            .to_rgba8();
        assert_eq!(
            actual, expected,
            "workers={workers}: the job must read the original"
        );
    }
}

#[test]
fn compression_workers_process_a_batch_without_idle_delay() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let photo_path = dir
        .path()
        .join("photos/VRChat_2026-10-21_21-14-03.221_4x3.png");
    write_photo(&photo_path);
    daemon.run_once().unwrap();
    std::fs::remove_file(&photo_path).unwrap();

    {
        let db = daemon.db.lock().unwrap();
        let photo = db
            .photo_by_path(&photo_path.to_string_lossy())
            .unwrap()
            .expect("indexed photo");
        let profile = db
            .ensure_default_profiles()
            .unwrap()
            .into_iter()
            .next()
            .expect("default profile");
        db.enqueue_job(photo.id, &profile.id, "keep").unwrap();
        db.enqueue_job(photo.id, &profile.id, "keep").unwrap();
    }

    daemon.settings.compression_workers = 2;
    let events = daemon.events.subscribe();
    let started = Instant::now();
    daemon.start_workers();
    let mut failures = 0;
    while failures < 2 {
        let event = events
            .recv_timeout(Duration::from_millis(300))
            .expect("worker idle delay");
        if let Event::CompressionProgress {
            status: JobStatus::Failed,
            ..
        } = event
        {
            failures += 1;
        }
    }
    assert!(
        started.elapsed() < Duration::from_millis(300),
        "batch took {:?}",
        started.elapsed()
    );
    daemon.stop_workers();
}

#[test]
fn compress_pending_processes_every_profile_for_one_photo() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = manual_test_daemon(&dir);
    daemon.run_once().unwrap();
    let db = Db::open(dir.path().join("library.db")).unwrap();
    let photo = db
        .photo_by_path(original_path(&dir).to_str().unwrap())
        .unwrap()
        .unwrap();
    save_batch_profile(
        &db,
        "webp",
        vrc_photon_core::model::CompressionFormat::Webp,
        "_webp",
        None,
    );
    save_batch_profile(
        &db,
        "jpeg",
        vrc_photon_core::model::CompressionFormat::Jpeg,
        "_jpeg",
        None,
    );
    db.enqueue_job(photo.id, "webp", "keep").unwrap();
    db.enqueue_job(photo.id, "jpeg", "keep").unwrap();

    daemon.compress_pending().unwrap();

    assert!(
        daemon.running.load(Ordering::Relaxed),
        "compress_pending must not start and stop the worker set"
    );
    assert!(daemon.workers.is_empty());
    let db = Db::open(dir.path().join("library.db")).unwrap();
    let jobs = db.list_jobs(10).unwrap();
    assert_eq!(jobs.len(), 2);
    assert!(jobs.iter().all(|job| job.status == QueueStatus::Completed));
    assert_eq!(db.derivatives_for_photo(photo.id).unwrap().len(), 2);
}

#[test]
fn batch_keeps_the_original_source_after_a_replace_job() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = manual_test_daemon(&dir);
    daemon.run_once().unwrap();
    let db = Db::open(dir.path().join("library.db")).unwrap();
    let photo = db
        .photo_by_path(original_path(&dir).to_str().unwrap())
        .unwrap()
        .unwrap();
    save_batch_profile(
        &db,
        "small",
        vrc_photon_core::model::CompressionFormat::Webp,
        "_small",
        Some((8, 8)),
    );
    save_batch_profile(
        &db,
        "full",
        vrc_photon_core::model::CompressionFormat::Webp,
        "_full",
        None,
    );
    db.enqueue_job(photo.id, "small", "replace").unwrap();
    db.enqueue_job(photo.id, "full", "keep").unwrap();

    daemon.compress_pending().unwrap();

    let db = Db::open(dir.path().join("library.db")).unwrap();
    let derivatives = db.derivatives_for_photo(photo.id).unwrap();
    let small = derivatives
        .iter()
        .find(|derivative| derivative.profile_id.as_deref() == Some("small"))
        .unwrap();
    let full = derivatives
        .iter()
        .find(|derivative| derivative.profile_id.as_deref() == Some("full"))
        .unwrap();
    assert_eq!((small.width, small.height), (8, 5));
    assert_eq!(
        (full.width, full.height),
        (16, 10),
        "second job read the derivative"
    );
}

#[test]
fn batch_continues_after_one_profile_fails() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = manual_test_daemon(&dir);
    daemon.run_once().unwrap();
    let blocked = dir.path().join("photos/not-a-directory");
    std::fs::write(&blocked, b"x").unwrap();
    let db = Db::open(dir.path().join("library.db")).unwrap();
    let photo = db
        .photo_by_path(original_path(&dir).to_str().unwrap())
        .unwrap()
        .unwrap();
    save_batch_profile(
        &db,
        "good",
        vrc_photon_core::model::CompressionFormat::Webp,
        "_good",
        None,
    );
    save_batch_profile(
        &db,
        "bad",
        vrc_photon_core::model::CompressionFormat::Webp,
        "_bad",
        None,
    );
    db.enqueue_job(photo.id, "good", "keep").unwrap();
    db.enqueue_job_with_output_dir(photo.id, "bad", "keep", Some(blocked.to_str().unwrap()))
        .unwrap();

    daemon.compress_pending().unwrap();

    let db = Db::open(dir.path().join("library.db")).unwrap();
    let jobs = db.list_jobs(10).unwrap();
    let good = jobs
        .iter()
        .find(|job| job.profile_id.as_deref() == Some("good"))
        .unwrap();
    let bad = jobs
        .iter()
        .find(|job| job.profile_id.as_deref() == Some("bad"))
        .unwrap();
    assert_eq!(good.status, QueueStatus::Completed);
    assert_eq!(bad.status, QueueStatus::Failed);
    assert_eq!(db.derivatives_for_photo(photo.id).unwrap().len(), 1);
}

#[test]
fn batch_decode_failure_fails_every_job() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = manual_test_daemon(&dir);
    daemon.run_once().unwrap();
    let db = Db::open(dir.path().join("library.db")).unwrap();
    let photo = db
        .photo_by_path(original_path(&dir).to_str().unwrap())
        .unwrap()
        .unwrap();
    save_batch_profile(
        &db,
        "first",
        vrc_photon_core::model::CompressionFormat::Webp,
        "_first",
        None,
    );
    save_batch_profile(
        &db,
        "second",
        vrc_photon_core::model::CompressionFormat::Webp,
        "_second",
        None,
    );
    db.enqueue_job(photo.id, "first", "keep").unwrap();
    db.enqueue_job(photo.id, "second", "keep").unwrap();
    std::fs::remove_file(original_path(&dir)).unwrap();

    daemon.compress_pending().unwrap();

    let db = Db::open(dir.path().join("library.db")).unwrap();
    let jobs = db.list_jobs(10).unwrap();
    assert_eq!(jobs.len(), 2);
    assert!(jobs.iter().all(|job| job.status == QueueStatus::Failed));
    assert!(jobs.iter().all(|job| job.error.is_some()));
}

#[test]
fn forced_deletion_cancels_the_rest_of_the_batch() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = manual_test_daemon(&dir);
    daemon.run_once().unwrap();
    let db = Db::open(dir.path().join("library.db")).unwrap();
    let photo = db
        .photo_by_path(original_path(&dir).to_str().unwrap())
        .unwrap()
        .unwrap();
    save_batch_profile(
        &db,
        "first",
        vrc_photon_core::model::CompressionFormat::Webp,
        "_first",
        None,
    );
    save_batch_profile(
        &db,
        "second",
        vrc_photon_core::model::CompressionFormat::Webp,
        "_second",
        None,
    );
    db.enqueue_job(photo.id, "first", "keep").unwrap();
    db.enqueue_job(photo.id, "second", "keep").unwrap();
    let jobs = db.next_pending_photo_jobs().unwrap();
    assert_eq!(jobs.len(), 2);
    let request = db
        .request_deletion(&[photo.id], DeletionReason::Manual, false)
        .unwrap()[0];
    assert!(db.force_deletion(request).unwrap());

    let outcomes = process_photo_batch(
        &daemon.db,
        &jobs,
        Some(&daemon.thumbnail_dir),
        &daemon.compression_budget,
    );

    assert_eq!(outcomes[0].status, QueueStatus::Completed);
    assert_eq!(outcomes[1].status, QueueStatus::Cancelled);
    let jobs = db.list_jobs(10).unwrap();
    let cancelled = jobs
        .iter()
        .find(|job| job.status == QueueStatus::Cancelled)
        .unwrap();
    assert!(cancelled.finished_at.is_some());
    assert!(db.claim_deletion(request).unwrap().is_some());
}

#[test]
fn scan_root_command_scopes_missing_detection() {
    let dir = tempfile::tempdir().unwrap();
    let root_a = dir.path().join("photos");
    let root_b = dir.path().join("imported");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&root_a).unwrap();
    std::fs::create_dir_all(&root_b).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let b_path = root_b.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&b_path);
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(root_a.clone()), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    save_daemon_settings(&daemon, |settings| {
        settings
            .photo_roots
            .push(root_b.to_string_lossy().to_string());
    });
    {
        let db = Db::open(&db_path).unwrap();
        db.request_daemon_command(DaemonCommand::Rescan).unwrap();
    }
    daemon.poll_commands().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert_eq!(db.count_photos().unwrap(), 1);
    std::fs::remove_file(&b_path).unwrap();
    let a_path = root_a.join("VRChat_2026-10-21_21-25-00.000_4x3.png");
    // Distinct pixels and dimensions keep this from being a rebind
    // candidate for the removed B photo.
    image::RgbaImage::from_pixel(8, 5, image::Rgba([1, 2, 3, 255]))
        .save(&a_path)
        .unwrap();
    assert!(
        db.request_daemon_scan_root(root_a.to_str().unwrap())
            .unwrap()
    );
    daemon.poll_commands().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert_eq!(
        db.photo_by_path(b_path.to_str().unwrap())
            .unwrap()
            .unwrap()
            .status,
        PhotoStatus::Present
    );
    assert!(
        db.photo_by_path(a_path.to_str().unwrap())
            .unwrap()
            .is_some()
    );

    db.request_daemon_command(DaemonCommand::Rescan).unwrap();
    daemon.poll_commands().unwrap();
    assert_eq!(
        db.photo_by_path(b_path.to_str().unwrap())
            .unwrap()
            .unwrap()
            .status,
        PhotoStatus::Missing
    );
}

#[test]
fn scan_root_command_indexes_an_unconfigured_directory() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let imported = dir.path().join("imported");
    std::fs::create_dir_all(&imported).unwrap();
    let path = imported.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&path);
    {
        let db = daemon.db.lock().unwrap();
        db.request_daemon_scan_root(imported.to_str().unwrap())
            .unwrap();
    }
    daemon.poll_commands().unwrap();

    let db = daemon.db.lock().unwrap();
    assert!(db.photo_by_path(path.to_str().unwrap()).unwrap().is_some());
    assert_eq!(db.count_photos().unwrap(), 1);
}

#[test]
fn scan_root_command_enqueues_auto_jobs_without_backlog() {
    let dir = tempfile::tempdir().unwrap();
    let root_a = dir.path().join("photos");
    let root_b = dir.path().join("imported");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&root_a).unwrap();
    std::fs::create_dir_all(&root_b).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    write_photo(&root_b.join("VRChat_2026-10-21_21-24-00.000_4x3.png"));
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(root_a), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    save_daemon_settings(&daemon, |settings| {
        settings.auto_compress_backlog = false;
        settings
            .photo_roots
            .push(root_b.to_string_lossy().to_string());
    });
    {
        let db = Db::open(&db_path).unwrap();
        db.request_daemon_scan_root(root_b.to_str().unwrap())
            .unwrap();
    }
    daemon.poll_commands().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert_eq!(db.count_jobs(QueueStatus::Pending).unwrap(), 1);
}

#[test]
fn startup_scan_enqueues_imported_photos_before_backlog() {
    let dir = tempfile::tempdir().unwrap();
    let backlog_root = dir.path().join("photos");
    let import_root = dir.path().join("imported");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&backlog_root).unwrap();
    std::fs::create_dir_all(&import_root).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let backlog_photo = backlog_root.join("VRChat_2026-10-21_21-14-03.221_4x3.png");
    let imported_photo = import_root.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&backlog_photo);
    write_photo(&imported_photo);

    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(backlog_root.clone()), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    daemon.set_ipc_endpoint(None);
    save_daemon_settings(&daemon, |settings| {
        settings.auto_compress = true;
        settings.auto_compress_backlog = false;
        settings.auto_compress_immediate = true;
        settings.photo_roots = vec![
            backlog_root.to_string_lossy().to_string(),
            import_root.to_string_lossy().to_string(),
        ];
    });
    {
        let db = Db::open(&db_path).unwrap();
        db.request_daemon_scan_root(import_root.to_str().unwrap())
            .unwrap();
    }

    daemon.startup_scan().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert_eq!(db.count_photos().unwrap(), 2);
    let imported_id = db
        .photo_by_path(imported_photo.to_str().unwrap())
        .unwrap()
        .unwrap()
        .id;
    let backlog_id = db
        .photo_by_path(backlog_photo.to_str().unwrap())
        .unwrap()
        .unwrap()
        .id;
    let jobs = db.list_jobs(10).unwrap();
    assert_eq!(
        jobs.iter()
            .filter(|job| job.photo_id == imported_id)
            .count(),
        1
    );
    assert_eq!(
        jobs.iter().filter(|job| job.photo_id == backlog_id).count(),
        0
    );
    assert_eq!(jobs[0].status, QueueStatus::Pending);
}

#[test]
fn scan_root_finished_event_reports_auto_enqueued_jobs() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    write_photo(&photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png"));

    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir.clone()), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    daemon.set_ipc_endpoint(None);
    let events = daemon.events.subscribe();
    daemon.scan_root_photos(photo_dir.clone()).unwrap();

    let mut finished = None;
    while let Ok(event) = events.try_recv() {
        if matches!(event, Event::ScanFinished { .. }) {
            finished = Some(event);
        }
    }
    match finished {
        Some(Event::ScanFinished {
            auto_enqueued,
            target,
            ..
        }) => {
            assert_eq!(auto_enqueued, 1);
            assert_eq!(target.as_deref(), Some(photo_dir.to_str().unwrap()));
        }
        _ => panic!("scan did not finish"),
    }
}

#[test]
fn unchanged_rescan_does_not_bump_the_library_revision() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    write_photo(
        &dir.path()
            .join("photos/VRChat_2026-10-21_21-14-03.221_4x3.png"),
    );
    daemon.run_once().unwrap();
    let revision = {
        let db = daemon.db.lock().unwrap();
        db.library_revision().unwrap()
    };
    daemon.run_once().unwrap();
    let db = daemon.db.lock().unwrap();
    assert_eq!(db.library_revision().unwrap(), revision);
}

#[test]
fn scan_root_photos_reports_a_single_root() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    write_photo(
        &dir.path()
            .join("photos/VRChat_2026-10-21_21-14-03.221_4x3.png"),
    );
    let events = daemon.events.subscribe();
    let root = dir.path().join("photos");
    let report = daemon.scan_root_photos(root).unwrap();
    assert_eq!(report.indexed, 1);
    match events.try_recv().unwrap() {
        Event::ScanStarted { roots, .. } => assert_eq!(roots, 1),
        other => panic!("unexpected event: {other:?}"),
    }
}

#[test]
fn scan_root_missing_path_records_failure_and_keeps_other_photos() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let photo = dir
        .path()
        .join("photos/VRChat_2026-10-21_21-14-03.221_4x3.png");
    write_photo(&photo);
    let root = dir.path().join("photos");
    daemon.scan_root_photos(root).unwrap();

    let missing = dir.path().join("does-not-exist");
    let report = daemon.scan_root_photos(missing.clone()).unwrap();
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].path, missing);

    let not_a_dir = dir.path().join("not-a-dir.png");
    write_photo(&not_a_dir);
    let report = daemon.scan_root_photos(not_a_dir.clone()).unwrap();
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].path, not_a_dir);

    let db = daemon.db.lock().unwrap();
    let indexed = db.photo_by_path(photo.to_str().unwrap()).unwrap().unwrap();
    assert_eq!(indexed.status, PhotoStatus::Present);
    let errors = db.daemon_errors().unwrap();
    assert!(
        errors
            .iter()
            .any(|error| error.operation == DaemonOperation::Scan)
    );
}

#[test]
fn scan_root_sends_new_photos_to_the_thumbnail_queue() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let photo = dir
        .path()
        .join("photos/VRChat_2026-10-21_21-14-03.221_4x3.png");
    write_photo(&photo);
    let (tx, rx) = std::sync::mpsc::channel();
    daemon.thumbnail_tx = Some(tx);
    let events = daemon.events.subscribe();
    let root = dir.path().join("photos");
    let report = daemon.scan_root_photos(root.clone()).unwrap();
    assert_eq!(report.new_ids.len(), 1);

    assert_eq!(
        rx.try_recv().unwrap(),
        ThumbnailRequest::Photo(report.new_ids[0])
    );
    assert!(rx.try_recv().is_err());

    let mut finished = None;
    while let Ok(event) = events.try_recv() {
        if let Event::ScanFinished { .. } = event {
            finished = Some(event);
        }
    }
    match finished {
        Some(Event::ScanFinished {
            failures,
            failed,
            target,
            ..
        }) => {
            assert_eq!(failures, 0);
            assert!(!failed);
            assert_eq!(target.as_deref(), Some(root.to_str().unwrap()));
        }
        _ => panic!("scan did not finish"),
    }
}

#[test]
fn thumbnail_settings_change_wakes_the_worker() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let (tx, rx) = std::sync::mpsc::channel();
    daemon.thumbnail_tx = Some(tx);
    daemon.thumbnail_cache_limit.store(1, Ordering::Relaxed);
    daemon.refresh_compression_settings();
    assert!(matches!(rx.try_recv().unwrap(), ThumbnailRequest::Refresh));
}

#[test]
fn notify_workers_wakes_hash_and_thumbnail() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let (tx, rx) = std::sync::mpsc::channel();
    daemon.thumbnail_tx = Some(tx);
    let before = daemon.hash_wake.generation();
    daemon.notify_workers();
    assert!(daemon.hash_wake.generation() > before);
    assert!(matches!(rx.try_recv().unwrap(), ThumbnailRequest::Refresh));
}

#[test]
fn scan_photos_publishes_started_progress_and_finished() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    write_photo(
        &dir.path()
            .join("photos/VRChat_2026-10-21_21-14-03.221_4x3.png"),
    );
    let events = daemon.events.subscribe();
    daemon.run_once().unwrap();

    assert!(matches!(
        events.try_recv().unwrap(),
        Event::ScanStarted { .. }
    ));
    let mut finished = None;
    while let Ok(event) = events.try_recv() {
        if let Event::ScanFinished { .. } = event {
            finished = Some(event);
        }
    }
    match finished {
        Some(Event::ScanFinished { indexed, .. }) => assert_eq!(indexed, 1),
        _ => panic!("scan did not finish"),
    }
}

#[test]
fn scan_photos_stops_between_roots_when_stopping() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let second = dir.path().join("photos2");
    std::fs::create_dir_all(&second).unwrap();
    write_photo(&second.join("VRChat_2026-10-21_21-14-04.221_4x3.png"));
    daemon.settings.photo_roots = vec![
        dir.path().join("photos").to_string_lossy().to_string(),
        second.to_string_lossy().to_string(),
    ];
    daemon.running.store(false, Ordering::Relaxed);

    let report = daemon.scan_photos().unwrap();
    assert_eq!(report.indexed, 0);
    assert_eq!(daemon.db.lock().unwrap().count_photos().unwrap(), 0);
}

#[test]
fn set_paused_publishes_status_changed_and_wakes() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = test_daemon(&dir);
    let events = daemon.events.subscribe();
    daemon.set_paused(true);
    assert!(matches!(
        events.try_recv().unwrap(),
        Event::StatusChanged { .. }
    ));
    assert!(daemon.wake.is_pending());
    assert!(daemon.paused.load(Ordering::Relaxed));
}

#[test]
fn supervised_restart_updates_daemon_status() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = test_daemon(&dir);
    daemon.record_supervised_restart("compress-0", "worker boom");
    let status = daemon.status.lock().unwrap();
    assert_eq!(status.worker_restarts, 1);
    assert_eq!(
        status.last_worker_error.as_deref(),
        Some("compress-0: worker boom")
    );
}

#[test]
fn paused_loop_keeps_heartbeat_status_and_log_ingestion() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);
    let log_path = dir.path().join("logs/output_log_2026-10-21_21-00-00.txt");
    std::fs::write(
        &log_path,
        concat!(
            "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)\n",
            "2026.10.21 21:23:13 Debug      -  [Behaviour] Joining or Creating Room: Test World\n",
            "2026.10.21 21:23:20 Debug      -  [Behaviour] OnPlayerJoined Mio (usr_a)\n",
        ),
    )
    .unwrap();
    {
        let db = daemon.db.lock().unwrap();
        db.set_setting("daemon_heartbeat", "1").unwrap();
    }

    daemon.set_paused(true);
    daemon.last_log_poll = Instant::now() - LOG_INTERVAL;
    daemon.last_heartbeat = Instant::now() - Duration::from_secs(11);
    daemon.run_loop_iteration();

    let db = daemon.db.lock().unwrap();
    assert!(
        db.log_offset(log_path.to_str().unwrap()).unwrap().is_some(),
        "log ingestion must continue while paused"
    );
    assert_ne!(
        db.get_setting("daemon_heartbeat").unwrap().as_deref(),
        Some("1"),
        "heartbeat must continue while paused"
    );
    drop(db);
    let status = daemon.status.lock().unwrap();
    assert!(status.paused, "pause must stay visible in status");
    assert_eq!(
        status.last_log,
        Some(log_path.to_string_lossy().to_string())
    );
    assert!(status.watching);
}

#[test]
fn session_refresh_does_not_touch_counts() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = test_daemon(&dir);
    {
        let mut status = daemon.status.lock().unwrap();
        status.photo_count = 123;
        status.pending_jobs = 4;
        status.pending_sends = 5;
        status.pending_tagging_jobs = 6;
    }
    daemon.refresh_session_status().unwrap();
    let status = daemon.status.lock().unwrap();
    assert_eq!(status.photo_count, 123);
    assert_eq!(status.pending_jobs, 4);
    assert_eq!(status.pending_sends, 5);
    assert_eq!(status.pending_tagging_jobs, 6);
}

#[test]
fn counts_refresh_does_not_touch_session_fields() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = test_daemon(&dir);
    {
        let mut status = daemon.status.lock().unwrap();
        status.world_name = Some("keep".to_string());
        status.players = 7;
    }
    daemon.refresh_counts().unwrap();
    let status = daemon.status.lock().unwrap();
    assert_eq!(status.world_name.as_deref(), Some("keep"));
    assert_eq!(status.players, 7);
}

#[test]
fn status_publish_is_skipped_when_nothing_changed() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let events = daemon.events.subscribe();
    daemon.publish_status_if_changed();
    daemon.publish_status_if_changed();
    assert_eq!(
        events
            .try_iter()
            .filter(|event| matches!(event, Event::StatusChanged { .. }))
            .count(),
        1
    );
}

#[test]
fn status_publish_fires_when_a_field_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let events = daemon.events.subscribe();
    daemon.publish_status_if_changed();
    daemon.set_paused(true);
    while events.try_recv().is_ok() {}
    daemon.publish_status_if_changed();
    assert!(
        events
            .try_iter()
            .any(|event| matches!(event, Event::StatusChanged { .. }))
    );
}

#[test]
fn loop_refreshes_counts_only_when_due() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);

    daemon.last_counts_status = Instant::now();
    {
        let mut status = daemon.status.lock().unwrap();
        status.photo_count = 42;
    }
    daemon.run_loop_iteration();
    assert_eq!(daemon.status.lock().unwrap().photo_count, 42);

    daemon.last_counts_status = Instant::now() - COUNTS_STATUS_INTERVAL;
    daemon.run_loop_iteration();
    let expected = daemon.db.lock().unwrap().count_photos().unwrap();
    assert_eq!(daemon.status.lock().unwrap().photo_count, expected);
}

#[test]
fn min_wait_uses_the_earliest_deadline_and_caps_at_the_maximum() {
    assert_eq!(
        crate::lifecycle::min_wait(
            &[Duration::from_secs(5), Duration::from_secs(2)],
            Duration::from_secs(1)
        ),
        Duration::from_secs(1)
    );
    assert_eq!(
        crate::lifecycle::min_wait(
            &[Duration::from_millis(200), Duration::from_secs(5)],
            Duration::from_secs(1)
        ),
        Duration::from_millis(200)
    );
    assert_eq!(
        crate::lifecycle::min_wait(&[], Duration::from_secs(1)),
        Duration::from_secs(1)
    );
    assert_eq!(
        crate::lifecycle::min_wait(&[Duration::ZERO], Duration::from_secs(1)),
        Duration::from_millis(10)
    );
}

#[test]
fn headless_idle_wait_is_at_most_one_second() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);
    daemon.last_log_poll = Instant::now() - LOG_INTERVAL;
    assert!(daemon.next_loop_wait() <= Duration::from_secs(1));
}

#[test]
fn tray_idle_wait_is_at_most_five_hundred_milliseconds() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(true);
    assert!(daemon.next_loop_wait() <= Duration::from_millis(500));
}

#[test]
fn paused_idle_wait_does_not_spin_on_an_overdue_asset_check() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);
    let now = Instant::now();
    daemon.last_log_poll = now;
    daemon.last_presence_check = now;
    daemon.last_session_status = now;
    daemon.last_counts_status = now;
    daemon.last_settings_check = now;
    daemon.last_autostart_check = now;
    daemon.last_heartbeat = now;
    daemon.last_asset_check = now - Duration::from_secs(60);
    daemon.set_paused(true);
    assert!(daemon.next_loop_wait() >= Duration::from_millis(100));
}

#[test]
fn presence_check_runs_only_after_its_interval() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);

    daemon.last_presence_check = Instant::now() - PRESENCE_INTERVAL;
    daemon.run_loop_iteration();
    let after_due = daemon.last_presence_check;
    assert!(after_due.elapsed() < Duration::from_secs(1));

    daemon.run_loop_iteration();
    assert_eq!(daemon.last_presence_check, after_due);
}

#[test]
fn changing_the_log_dir_invalidates_the_locator() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.ingest_logs().unwrap();
    assert!(daemon.log_locator.is_some());

    save_daemon_settings(&daemon, |settings| {
        settings.log_dir = Some(dir.path().join("logs2").to_string_lossy().to_string());
    });
    daemon.reload_settings().unwrap();
    assert!(daemon.log_locator.is_none());
}

#[test]
fn paused_loop_collects_watcher_events_and_indexes_them_after_resume() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);
    let (tx, rx) = mpsc::channel();
    daemon.watcher_rx = Some(rx);
    let path = dir
        .path()
        .join("photos/VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&path);
    tx.send(Ok(notify::Event::new(notify::EventKind::Create(
        notify::event::CreateKind::File,
    ))
    .add_path(path.clone())))
        .unwrap();

    daemon.set_paused(true);
    daemon.run_loop_iteration();

    // The watcher event is collected, but indexing is heavy work that
    // waits for resume.
    assert_eq!(daemon.dirty_paths, vec![path.clone()]);
    assert_eq!(daemon.db.lock().unwrap().count_photos().unwrap(), 0);

    daemon.set_paused(false);
    daemon.run_loop_iteration();

    assert!(
        daemon.dirty_paths.is_empty(),
        "the buffered event must be indexed, not lost"
    );
    assert_eq!(daemon.db.lock().unwrap().count_photos().unwrap(), 1);
    assert_eq!(daemon.status.lock().unwrap().photo_count, 1);
}

#[test]
fn paused_loop_defers_queued_commands_until_resume() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);
    write_photo(
        &dir.path()
            .join("photos/VRChat_2026-10-21_21-24-00.000_4x3.png"),
    );
    {
        let db = daemon.db.lock().unwrap();
        assert!(db.request_daemon_command(DaemonCommand::Rescan).unwrap());
    }

    daemon.set_paused(true);
    daemon.run_loop_iteration();
    assert_eq!(daemon.db.lock().unwrap().count_photos().unwrap(), 0);
    assert_eq!(daemon.deferred_commands.len(), 1);

    daemon.set_paused(false);
    daemon.run_loop_iteration();
    assert_eq!(daemon.db.lock().unwrap().count_photos().unwrap(), 1);
    assert!(daemon.deferred_commands.is_empty());
}

#[test]
fn shutdown_requeues_commands_deferred_while_paused() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);
    {
        let db = daemon.db.lock().unwrap();
        assert!(db.request_daemon_command(DaemonCommand::Rescan).unwrap());
    }
    daemon.set_paused(true);
    daemon.run_loop_iteration();
    assert_eq!(daemon.deferred_commands.len(), 1);

    daemon.shutdown();

    let commands = daemon.db.lock().unwrap().take_daemon_commands().unwrap();
    assert_eq!(commands, vec![DaemonCommand::Rescan]);
    assert!(daemon.deferred_commands.is_empty());
}

#[test]
fn toggling_pause_keeps_scan_progress_and_daemon_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    {
        let mut status = daemon.status.lock().unwrap();
        status.scanning = true;
        status.scan_phase = Some(IpcScanPhase::Walking);
        status.scan_processed = 7;
        status.scan_total = Some(10);
        status.scan_indexed = 3;
    }
    daemon
        .report_daemon_error(DaemonOperation::Scan, "boom", Some("detail"))
        .unwrap();

    for paused in [true, false] {
        daemon.set_paused(paused);
        let status = daemon.status.lock().unwrap();
        assert!(status.scanning);
        assert_eq!(status.scan_processed, 7);
        assert_eq!(status.scan_total, Some(10));
        assert_eq!(status.scan_indexed, 3);
        drop(status);
        let db = daemon.db.lock().unwrap();
        assert_eq!(db.daemon_errors().unwrap().len(), 1);
    }
}

#[test]
fn paused_daemon_serves_status_over_ipc() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let endpoint = Endpoint::for_tests(dir.path());
    daemon.set_ipc_endpoint(Some(endpoint.clone()));
    daemon.start_ipc();
    daemon.set_paused(true);

    let response = vrc_photon_ipc::client::request(
        &endpoint,
        vrc_photon_ipc::Request::Status,
        Duration::from_secs(1),
    )
    .unwrap();
    match response {
        vrc_photon_ipc::Response::Status { status } => {
            assert!(status.paused);
            assert_eq!(status.version, daemon.status.lock().unwrap().version);
        }
        other => panic!("unexpected response: {other:?}"),
    }
    daemon.ipc = None;
}

#[test]
fn report_daemon_error_publishes_changed_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let events = daemon.events.subscribe();
    daemon
        .report_daemon_error(DaemonOperation::Scan, "boom", None)
        .unwrap();
    assert!(matches!(
        events.try_recv().unwrap(),
        Event::DaemonErrorChanged {
            operation: Operation::Scan
        }
    ));
    daemon
        .report_daemon_error(DaemonOperation::Scan, "boom", None)
        .unwrap();
    assert!(events.try_recv().is_err());
}

#[test]
fn start_ipc_serves_the_daemon_status() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let endpoint = Endpoint::for_tests(dir.path());
    daemon.set_ipc_endpoint(Some(endpoint.clone()));
    daemon.start_ipc();

    let response = vrc_photon_ipc::client::request(
        &endpoint,
        vrc_photon_ipc::Request::Status,
        Duration::from_secs(1),
    )
    .unwrap();
    assert!(matches!(response, vrc_photon_ipc::Response::Status { .. }));
    daemon.ipc = None;
}

#[test]
fn reordering_profiles_does_not_change_the_auto_job() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("library.db")).unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo_id = db
        .upsert_photo(&vrc_photon_core::model::NewPhoto {
            root_id: root,
            rel_path: "a.png".to_string(),
            abs_path: "/photos/a.png".to_string(),
            file_name: "a.png".to_string(),
            captured_at_utc: 1_700_000_000,
            captured_at_local: "2026-10-21 21:14:03".to_string(),
            tz_offset_sec: 9 * 3600,
            width: 16,
            height: 10,
            file_size: 10,
            mtime: 1_700_000_000,
            blake3: None,
        })
        .unwrap();

    let make = |id: &str, name: &str| CompressionProfile {
        id: id.to_string(),
        name: name.to_string(),
        ..CompressionProfile::default()
    };
    let a = make("a", "A");
    let b = make("b", "B");
    let c = make("c", "C");
    for profile in [&a, &b, &c] {
        db.save_profile(profile).unwrap();
    }
    db.reorder_profiles(&[c.id.clone(), a.id.clone(), b.id.clone()])
        .unwrap();

    enqueue_auto_jobs(&db, &[photo_id], true, true).unwrap();

    let jobs = db.list_jobs(10).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].origin, CompressionOrigin::Auto);
    assert_eq!(jobs[0].profile_id, None);
    assert_eq!(enqueue_auto_jobs(&db, &[photo_id], true, true).unwrap(), 0);
}

#[test]
fn auto_compression_enqueues_without_any_profiles() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let photo = dir
        .path()
        .join("photos/VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&photo);
    daemon.run_once().unwrap();

    let db = daemon.db.lock().unwrap();
    let photo_id = db
        .photo_by_path(photo.to_str().unwrap())
        .unwrap()
        .unwrap()
        .id;
    assert!(db.list_profiles().unwrap().is_empty());

    assert_eq!(enqueue_auto_jobs(&db, &[photo_id], true, true).unwrap(), 1);
    let jobs = db.list_jobs(10).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].origin, CompressionOrigin::Auto);
    assert_eq!(jobs[0].profile_id, None);
}

#[test]
fn auto_jobs_use_replace_policy_and_waiting_status() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let original = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&original);

    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo_id = db
        .photo_by_path(original.to_str().unwrap())
        .unwrap()
        .unwrap()
        .id;
    enqueue_auto_jobs(&db, &[photo_id], false, false).unwrap();

    let job = db.list_jobs(1).unwrap().remove(0);
    assert_eq!(job.policy, "replace");
    assert_eq!(job.status, QueueStatus::Waiting);
    assert!(db.next_pending_job().unwrap().is_none());
}

#[test]
fn command_inserted_without_wake_is_still_processed() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);
    write_photo(
        &dir.path()
            .join("photos/VRChat_2026-10-21_21-24-00.000_4x3.png"),
    );

    {
        let db = Db::open(dir.path().join("library.db")).unwrap();
        db.request_daemon_command(DaemonCommand::Rescan).unwrap();
    }

    daemon.poll_commands().unwrap();
    assert_eq!(daemon.db.lock().unwrap().count_photos().unwrap(), 1);
}

#[test]
fn settings_reload_is_skipped_when_data_version_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);

    let version = {
        let db = daemon.db.lock().unwrap();
        db.data_version().unwrap()
    };
    daemon.last_data_version = Some(version);
    daemon.thumbnail_cache_limit.store(999, Ordering::Relaxed);
    daemon.last_settings_check = Instant::now() - SETTINGS_INTERVAL;
    daemon.refresh_periodic_settings();
    assert_eq!(daemon.thumbnail_cache_limit.load(Ordering::Relaxed), 999);
}

#[test]
fn settings_reload_applies_an_external_change() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.set_tray_enabled(false);

    let db_path = dir.path().join("library.db");
    let before = {
        let db = daemon.db.lock().unwrap();
        db.data_version().unwrap()
    };
    daemon.last_data_version = Some(before);

    let mut settings = AppSettings::load(&daemon.db.lock().unwrap()).unwrap();
    settings.thumbnail_cache_mb = 1;
    {
        let other = Db::open(&db_path).unwrap();
        settings.save(&other).unwrap();
    }

    daemon.last_settings_check = Instant::now() - SETTINGS_INTERVAL;
    daemon.refresh_periodic_settings();
    assert_eq!(
        daemon.thumbnail_cache_limit.load(Ordering::Relaxed),
        1024 * 1024
    );
}

#[test]
fn periodic_settings_refresh_applies_saved_thumbnail_limit() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    save_daemon_settings(&daemon, |settings| settings.thumbnail_cache_mb = 1);
    // The main loop calls this on every iteration, paused or not; make
    // the refresh due as it is one second after the previous one.
    daemon.last_settings_check = Instant::now() - Duration::from_secs(5);
    daemon.refresh_periodic_settings();
    assert_eq!(
        daemon.thumbnail_cache_limit.load(Ordering::Relaxed),
        1024 * 1024
    );
}

#[test]
fn periodic_settings_refresh_keeps_limit_until_due() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let before = daemon.thumbnail_cache_limit.load(Ordering::Relaxed);
    save_daemon_settings(&daemon, |settings| settings.thumbnail_cache_mb = 1);
    daemon.last_settings_check = Instant::now();
    daemon.refresh_periodic_settings();
    assert_eq!(daemon.thumbnail_cache_limit.load(Ordering::Relaxed), before);
}

#[test]
fn malformed_settings_abort_open_without_overwriting_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("library.db");
    let raw = r#"{"photo_roots":["/tmp/photos"],"language":"#;
    {
        let db = Db::open(&db_path).unwrap();
        db.set_setting("app_settings", raw).unwrap();
    }

    let error = match Daemon::open(&db_path, None, None) {
        Ok(_) => panic!("the daemon must not start with malformed settings"),
        Err(error) => error,
    };
    let message = format!("{error:#}");
    assert!(message.contains("cannot load settings"), "{message}");
    assert!(message.contains("json error"), "{message}");

    let db = Db::open(&db_path).unwrap();
    assert_eq!(
        db.get_setting("app_settings").unwrap().as_deref(),
        Some(raw),
        "open must not save defaults over the malformed row"
    );
    assert_eq!(
        db.get_setting("app_settings_corrupt").unwrap().as_deref(),
        Some(raw)
    );
}

#[test]
fn periodic_settings_refresh_keeps_in_memory_values_when_the_row_is_malformed() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    daemon.settings.auto_compress = false;
    daemon.settings.autostart = false;
    {
        let db = daemon.db.lock().unwrap();
        db.set_setting("app_settings", "{not json").unwrap();
    }
    daemon.last_settings_check = Instant::now() - Duration::from_secs(5);
    daemon.last_autostart_check = Instant::now() - Duration::from_secs(11);

    daemon.refresh_periodic_settings();

    assert!(!daemon.settings.auto_compress);
    assert!(!daemon.settings.autostart);
    let db = daemon.db.lock().unwrap();
    assert_eq!(
        db.get_setting("app_settings").unwrap().as_deref(),
        Some("{not json"),
        "refresh must not save settings"
    );
}

#[test]
fn refresh_compression_settings_applies_live_policy_and_waiting_mode() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let first = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&first);

    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir.clone()), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    daemon.run_once().unwrap();

    {
        let db = Db::open(&db_path).unwrap();
        let mut settings = AppSettings::load(&db).unwrap_or_default();
        settings.auto_compress = true;
        settings.auto_compress_immediate = false;
        settings.auto_compress_keep_original = false;
        settings.save(&db).unwrap();
    }
    let second = photo_dir.join("VRChat_2026-10-21_21-25-00.000_4x3.png");
    write_photo(&second);
    daemon.dirty_paths.push(second);
    daemon.index_dirty_paths().unwrap();

    assert!(!daemon.settings.auto_compress_immediate);
    assert!(!daemon.settings.auto_compress_keep_original);
    let db = Db::open(&db_path).unwrap();
    let job = db.list_jobs(1).unwrap().remove(0);
    assert_eq!(job.status, QueueStatus::Waiting);
    assert_eq!(job.policy, "replace");
}

#[test]
fn auto_compress_holds_waiting_jobs_until_released() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let original = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&original);

    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    save_daemon_settings(&daemon, |settings| {
        settings.auto_compress_backlog = true;
        settings.auto_compress_immediate = false;
    });
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db
        .photo_by_path(original.to_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(db.count_jobs(QueueStatus::Waiting).unwrap(), 1);
    daemon.compress_pending().unwrap();
    assert!(db.derivatives_for_photo(photo.id).unwrap().is_empty());

    db.release_waiting_jobs("keep").unwrap();
    daemon.compress_pending().unwrap();
    assert_eq!(db.derivatives_for_photo(photo.id).unwrap().len(), 1);
}

#[test]
fn auto_replace_failure_leaves_the_original_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let original = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&original);
    // Make the default output directory impossible to create.
    std::fs::write(photo_dir.join("compressed"), b"not a directory").unwrap();

    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    save_daemon_settings(&daemon, |settings| {
        settings.auto_compress_backlog = true;
        settings.auto_compress_keep_original = false;
    });
    daemon.run_once().unwrap();
    daemon.compress_pending().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert!(original.exists());
    let photo = db
        .photo_by_path(original.to_str().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(photo.status, PhotoStatus::Present);
    assert!(db.derivatives_for_photo(photo.id).unwrap().is_empty());
    assert_eq!(db.list_jobs(1).unwrap()[0].status, QueueStatus::Failed);
}

#[test]
fn end_to_end_auto_compress() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let photo_path = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&photo_path);

    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    save_daemon_settings(&daemon, |settings| {
        settings.auto_compress_backlog = true;
    });
    daemon.run_once().unwrap();
    daemon.compress_pending().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db
        .photo_by_path(photo_path.to_str().unwrap())
        .unwrap()
        .expect("photo");
    let derivatives = db.derivatives_for_photo(photo.id).unwrap();
    assert_eq!(derivatives.len(), 1);
    assert!(Path::new(&derivatives[0].path).is_file());
    assert!(derivatives[0].file_size > 0);
}

#[test]
fn auto_compression_always_writes_to_the_compressed_folder() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let photo_path = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&photo_path);

    let db_path = dir.path().join("library.db");
    {
        let db = Db::open(&db_path).unwrap();
        db.save_profile(&CompressionProfile {
            id: "custom-output".to_string(),
            name: "Custom output".to_string(),
            output_dir: Some("custom".to_string()),
            suffix: "_custom".to_string(),
            ..CompressionProfile::default()
        })
        .unwrap();
    }
    let mut daemon = Daemon::open(&db_path, Some(photo_dir.clone()), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    save_daemon_settings(&daemon, |settings| {
        settings.auto_compress_backlog = true;
    });
    daemon.run_once().unwrap();
    daemon.compress_pending().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db
        .photo_by_path(photo_path.to_str().unwrap())
        .unwrap()
        .expect("photo");
    let derivative = db
        .derivatives_for_photo(photo.id)
        .unwrap()
        .into_iter()
        .find(|derivative| derivative.origin == CompressionOrigin::Auto)
        .expect("auto derivative");
    let path = Path::new(&derivative.path);
    assert_eq!(
        path.parent().and_then(Path::file_name),
        Some(std::ffi::OsStr::new("compressed"))
    );
    assert!(path.is_file());
    assert!(!photo_dir.join("custom").exists());
    assert_eq!(derivative.profile_id, None);
}

#[test]
fn initial_scan_skips_backlog_when_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    write_photo(&photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png"));

    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir.clone()), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert_eq!(db.count_jobs(QueueStatus::Pending).unwrap(), 0);

    let new_path = photo_dir.join("VRChat_2026-10-21_21-25-00.000_4x3.png");
    write_photo(&new_path);
    daemon.dirty_paths.push(new_path);
    daemon.index_dirty_paths().unwrap();
    assert_eq!(db.count_jobs(QueueStatus::Pending).unwrap(), 1);
}

#[test]
fn initial_scan_enqueues_backlog_when_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    write_photo(&photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png"));

    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    save_daemon_settings(&daemon, |settings| {
        settings.auto_compress_backlog = true;
    });
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert_eq!(db.count_jobs(QueueStatus::Pending).unwrap(), 1);
}

#[test]
fn incremental_events_index_and_mark_missing() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir.clone()), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));

    let path = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    image::RgbaImage::from_pixel(4, 3, image::Rgba([10, 20, 30, 255]))
        .save(&path)
        .unwrap();
    daemon.dirty_paths.push(path.clone());
    daemon.index_dirty_paths().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db
        .photo_by_path(path.to_str().unwrap())
        .unwrap()
        .expect("indexed");
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Present);

    std::fs::remove_file(&path).unwrap();
    daemon.dirty_paths.push(path.clone());
    daemon.index_dirty_paths().unwrap();
    let photo = db
        .photo_by_path(path.to_str().unwrap())
        .unwrap()
        .expect("row");
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Missing);
}

#[test]
fn a_moved_file_keeps_the_same_row_and_tags() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir.clone()), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));

    let old_path = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&old_path);
    daemon.dirty_paths.push(old_path.clone());
    daemon.index_dirty_paths().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db
        .photo_by_path(old_path.to_str().unwrap())
        .unwrap()
        .expect("indexed");
    let photo_id = photo.id;
    db.set_photo_tags(photo_id, &["night".to_string()]).unwrap();
    let revision = db.library_revision().unwrap();

    let moved_dir = photo_dir.join("2026");
    std::fs::create_dir(&moved_dir).unwrap();
    let new_path = moved_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    std::fs::rename(&old_path, &new_path).unwrap();
    // A real watcher may deliver the destination before the source.
    daemon.dirty_paths.push(new_path.clone());
    daemon.dirty_paths.push(old_path.clone());
    daemon.index_dirty_paths().unwrap();

    let photo = db.photo_by_id(photo_id).unwrap().unwrap();
    assert_eq!(photo.abs_path, new_path.to_string_lossy());
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Present);
    assert_eq!(db.photo_tags(photo_id).unwrap(), vec!["night".to_string()]);
    assert_eq!(db.count_photos().unwrap(), 1);
    assert!(db.library_revision().unwrap() > revision);
}

#[test]
fn a_full_scan_rebinds_a_photo_moved_while_the_daemon_was_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir.clone()), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));

    let old_path = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&old_path);
    daemon.run_once().unwrap();
    let db = Db::open(&db_path).unwrap();
    let photo_id = db
        .photo_by_path(old_path.to_str().unwrap())
        .unwrap()
        .unwrap()
        .id;
    db.set_photo_tags(photo_id, &["night".to_string()]).unwrap();

    let moved_dir = photo_dir.join("2026");
    std::fs::create_dir(&moved_dir).unwrap();
    let new_path = moved_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    std::fs::rename(&old_path, &new_path).unwrap();
    daemon.run_once().unwrap();

    let photo = db.photo_by_id(photo_id).unwrap().unwrap();
    assert_eq!(photo.abs_path, new_path.to_string_lossy());
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Present);
    assert_eq!(db.photo_tags(photo_id).unwrap(), vec!["night".to_string()]);
    assert_eq!(db.count_photos().unwrap(), 1);
}

#[test]
fn a_renamed_file_keeps_the_same_row() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir.clone()), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));

    let old_path = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&old_path);
    daemon.dirty_paths.push(old_path.clone());
    daemon.index_dirty_paths().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db
        .photo_by_path(old_path.to_str().unwrap())
        .unwrap()
        .expect("indexed");
    let photo_id = photo.id;
    let captured_at = photo.captured_at_utc;
    db.set_photo_tags(photo_id, &["night".to_string()]).unwrap();

    let new_path = photo_dir.join("renamed.png");
    std::fs::rename(&old_path, &new_path).unwrap();
    daemon.dirty_paths.push(new_path.clone());
    daemon.dirty_paths.push(old_path.clone());
    daemon.index_dirty_paths().unwrap();

    let photo = db.photo_by_id(photo_id).unwrap().unwrap();
    assert_eq!(photo.abs_path, new_path.to_string_lossy());
    assert_eq!(photo.captured_at_utc, captured_at);
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Present);
    assert_eq!(db.photo_tags(photo_id).unwrap(), vec!["night".to_string()]);
    assert_eq!(db.count_photos().unwrap(), 1);
}

#[test]
fn a_move_between_configured_roots_keeps_the_same_row() {
    let dir = tempfile::tempdir().unwrap();
    let first_root = dir.path().join("photos");
    let second_root = dir.path().join("imported");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&first_root).unwrap();
    std::fs::create_dir_all(&second_root).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();

    let old_path = first_root.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&old_path);
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(first_root.clone()), Some(log_dir)).unwrap();
    daemon.settings.photo_roots = vec![
        first_root.to_string_lossy().to_string(),
        second_root.to_string_lossy().to_string(),
    ];
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db
        .photo_by_path(old_path.to_str().unwrap())
        .unwrap()
        .unwrap();
    let photo_id = photo.id;
    let new_path = second_root.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    std::fs::rename(&old_path, &new_path).unwrap();
    daemon.run_once().unwrap();

    let photo = db.photo_by_id(photo_id).unwrap().unwrap();
    assert_eq!(photo.abs_path, new_path.to_string_lossy());
    assert_eq!(
        photo.root_id,
        db.add_root(second_root.to_str().unwrap(), "reference")
            .unwrap()
    );
    assert_eq!(db.count_photos().unwrap(), 1);
}

#[test]
fn deleting_the_original_promotes_an_existing_auto_derivative() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir.clone()), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));

    let original = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&original);
    daemon.dirty_paths.push(original.clone());
    daemon.index_dirty_paths().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db
        .photo_by_path(original.to_str().unwrap())
        .unwrap()
        .expect("indexed");
    let compressed = photo_dir.join("compressed").join("kept.webp");
    std::fs::create_dir_all(compressed.parent().unwrap()).unwrap();
    std::fs::copy(&original, &compressed).unwrap();
    let derivative_id = db
        .add_derivative(
            photo.id,
            compressed.to_str().unwrap(),
            "webp",
            16,
            10,
            std::fs::metadata(&compressed).unwrap().len(),
            None,
            CompressionOrigin::Auto,
            "keep",
        )
        .unwrap();

    std::fs::remove_file(&original).unwrap();
    daemon.dirty_paths.push(original.clone());
    daemon.index_dirty_paths().unwrap();

    let photo = db.photo_by_id(photo.id).unwrap().unwrap();
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Replaced);
    assert_eq!(photo.display_derivative_id, Some(derivative_id));
    assert_eq!(photo.effective_path(), compressed.to_str().unwrap());
    assert_eq!(db.count_photos().unwrap(), 1);
}

#[test]
fn missing_original_promotes_the_auto_derivative() {
    let dir = tempfile::tempdir().unwrap();
    let photo_dir = dir.path().join("photos");
    let log_dir = dir.path().join("logs");
    std::fs::create_dir_all(&photo_dir).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(photo_dir.clone()), Some(log_dir)).unwrap();
    daemon.set_thumbnail_dir(dir.path().join("thumbnails"));

    let original = photo_dir.join("VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&original);
    daemon.dirty_paths.push(original.clone());
    daemon.index_dirty_paths().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db
        .photo_by_path(original.to_str().unwrap())
        .unwrap()
        .expect("indexed");
    let auto_path = dir.path().join("derivatives/auto.webp");
    let manual_path = dir.path().join("derivatives/manual.webp");
    std::fs::create_dir_all(auto_path.parent().unwrap()).unwrap();
    std::fs::write(&auto_path, b"auto").unwrap();
    std::fs::write(&manual_path, b"manual").unwrap();
    let auto = db
        .add_derivative(
            photo.id,
            auto_path.to_str().unwrap(),
            "webp",
            16,
            10,
            5,
            None,
            CompressionOrigin::Auto,
            "keep",
        )
        .unwrap();
    // A larger manual derivative id must not win.
    let manual = db
        .add_derivative(
            photo.id,
            manual_path.to_str().unwrap(),
            "webp",
            16,
            10,
            8,
            Some("manual"),
            CompressionOrigin::Manual,
            "keep",
        )
        .unwrap();
    assert_ne!(auto, manual);

    std::fs::remove_file(&original).unwrap();
    daemon.dirty_paths.push(original.clone());
    daemon.index_dirty_paths().unwrap();

    let photo = db.photo_by_id(photo.id).unwrap().unwrap();
    assert_eq!(photo.display_derivative_id, Some(auto));
    assert_ne!(photo.display_derivative_id, Some(manual));
    assert!(auto_path.is_file());
}

#[test]
fn manual_job_with_replace_policy_does_not_change_the_display() {
    let mut ctx = DaemonAiTest::new();
    let (photo_id, _path) = ctx.add_photo_and_scan();
    {
        let db = ctx.db.lock().unwrap();
        db.save_profile(&CompressionProfile {
            id: "manual".to_string(),
            name: "Manual".to_string(),
            ..CompressionProfile::default()
        })
        .unwrap();
        db.enqueue_job(photo_id, "manual", "replace").unwrap();
    }

    ctx.daemon.compress_pending().unwrap();

    let db = ctx.db.lock().unwrap();
    let photo = db.photo_by_id(photo_id).unwrap().unwrap();
    assert_eq!(photo.status, PhotoStatus::Present);
    assert_eq!(photo.display_derivative_id, None);
    let derivative = db.derivatives_for_photo(photo_id).unwrap().remove(0);
    assert_eq!(derivative.origin, CompressionOrigin::Manual);
    assert_eq!(derivative.policy, "keep");
}

#[test]
fn failed_job_blocks_deletion_until_retry_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = replace_test_daemon(&dir);
    let db_path = dir.path().join("library.db");
    let original = original_path(&dir);
    daemon.run_once().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db
        .photo_by_path(original.to_str().unwrap())
        .unwrap()
        .unwrap();
    db.save_profile(&CompressionProfile {
        id: "manual".to_string(),
        name: "Manual".to_string(),
        ..CompressionProfile::default()
    })
    .unwrap();
    // Enqueue the failing manual job before the auto replacement runs. The
    // batch processes the auto job first, then this one fails.
    let blocked_output = dir.path().join("photos/blocked");
    std::fs::write(&blocked_output, b"not a directory").unwrap();
    db.enqueue_job_with_output_dir(
        photo.id,
        "manual",
        "replace",
        Some(blocked_output.to_str().unwrap()),
    )
    .unwrap();
    drop(db);

    daemon.compress_pending().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db.photo_by_id(photo.id).unwrap().unwrap();
    assert!(original.exists(), "a failed job must keep the original");
    assert_eq!(photo.status, PhotoStatus::Replaced);
    let request = db.active_deletion_request(photo.id).unwrap().unwrap();
    assert_eq!(request.status, DeletionStatus::Blocked);
    assert_eq!(request.blocked_reason.as_deref(), Some("job_failed"));

    std::fs::remove_file(&blocked_output).unwrap();
    let failed = db.list_jobs_by_status(QueueStatus::Failed, 10).unwrap();
    assert_eq!(failed.len(), 1);
    db.retry_job(&failed[0].id).unwrap();
    drop(db);
    daemon.compress_pending().unwrap();

    let db = Db::open(&db_path).unwrap();
    assert!(!original.exists());
    assert!(db.active_deletion_request(photo.id).unwrap().is_none());
    assert_eq!(db.count_jobs(QueueStatus::Failed).unwrap(), 0);
}

#[test]
fn auto_replace_compression_keeps_the_photo_visible() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = replace_test_daemon(&dir);
    let db_path = dir.path().join("library.db");
    let original = original_path(&dir);
    daemon.run_once().unwrap();
    daemon.compress_pending().unwrap();

    let db = Db::open(&db_path).unwrap();
    let photo = db
        .photo_by_path(original.to_str().unwrap())
        .unwrap()
        .expect("indexed");

    let photo = db.photo_by_id(photo.id).unwrap().unwrap();
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Replaced);
    assert!(photo.display_derivative_id.is_some());
    assert!(
        photo
            .display_path
            .as_deref()
            .is_some_and(|path| Path::new(path).is_file())
    );
    assert!(!original.exists());
    assert_eq!(db.count_photos().unwrap(), 1);
    assert_eq!(db.stats().unwrap().photo_count, 1);

    // A subsequent full scan must not turn the replaced photo into a
    // missing one just because its original path is gone.
    daemon.run_once().unwrap();
    let photo = db.photo_by_id(photo.id).unwrap().unwrap();
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Replaced);

    let derivative_path = photo.display_path.clone().expect("display derivative");
    std::fs::remove_file(&derivative_path).unwrap();
    daemon.verify_replaced_assets().unwrap();
    let photo = db.photo_by_id(photo.id).unwrap().unwrap();
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Missing);
    assert!(photo.display_derivative_id.is_none());
    assert_eq!(db.count_photos().unwrap(), 0);
}

#[test]
fn verify_replaced_assets_keeps_live_files_and_marks_deleted_ones() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let photo_dir = dir.path().join("photos");
    let compressed = photo_dir.join("compressed");
    std::fs::create_dir_all(&compressed).unwrap();

    let live_derivative = compressed.join("live.webp");
    std::fs::write(&live_derivative, b"live").unwrap();
    let live_id = add_replaced_photo(
        &daemon,
        &photo_dir.join("VRChat_live.png"),
        &live_derivative,
    );
    let missing_id = add_replaced_photo(
        &daemon,
        &photo_dir.join("VRChat_missing.png"),
        &compressed.join("missing.webp"),
    );
    let no_display_id = add_replaced_photo(
        &daemon,
        &photo_dir.join("VRChat_no_display.png"),
        &compressed.join("no_display.webp"),
    );
    {
        let db = daemon.db.lock().unwrap();
        db.clear_display_derivative(no_display_id).unwrap();
    }
    let revision = {
        let db = daemon.db.lock().unwrap();
        db.library_revision().unwrap()
    };

    daemon.verify_replaced_assets().unwrap();

    let db = daemon.db.lock().unwrap();
    let live = db.photo_by_id(live_id).unwrap().unwrap();
    assert_eq!(live.status, vrc_photon_core::model::PhotoStatus::Replaced);
    assert!(live.display_derivative_id.is_some());
    for id in [missing_id, no_display_id] {
        let photo = db.photo_by_id(id).unwrap().unwrap();
        assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Missing);
        assert!(photo.display_derivative_id.is_none());
        assert!(photo.display_path.is_none());
    }
    assert_eq!(db.library_revision().unwrap(), revision + 1);
    assert_eq!(db.count_photos().unwrap(), 1);
}

#[test]
fn verify_replaced_assets_skips_photos_when_the_root_is_offline() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let offline = dir.path().join("offline-drive");
    let id = add_replaced_photo(
        &daemon,
        &offline.join("VRChat_offline.png"),
        &offline.join("compressed/offline.webp"),
    );
    let (revision, derivative_id) = {
        let db = daemon.db.lock().unwrap();
        (
            db.library_revision().unwrap(),
            db.photo_by_id(id).unwrap().unwrap().display_derivative_id,
        )
    };
    assert!(!offline.is_dir());

    daemon.verify_replaced_assets().unwrap();

    let db = daemon.db.lock().unwrap();
    let photo = db.photo_by_id(id).unwrap().unwrap();
    assert_eq!(photo.status, vrc_photon_core::model::PhotoStatus::Replaced);
    assert_eq!(photo.display_derivative_id, derivative_id);
    assert_eq!(db.library_revision().unwrap(), revision);
}

#[test]
fn verify_replaced_assets_releases_the_db_lock_while_checking_files() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = test_daemon(&dir);
    let root = dir.path().join("photos");
    let rows = vec![
        (
            1,
            Some(root.join("a.webp").to_string_lossy().to_string()),
            root.to_string_lossy().to_string(),
        ),
        (
            2,
            Some(root.join("b.webp").to_string_lossy().to_string()),
            root.to_string_lossy().to_string(),
        ),
    ];
    let db = Arc::clone(&daemon.db);
    let root_checks = std::cell::Cell::new(0);
    let missing = missing_display_derivatives(
        rows,
        |path| {
            assert!(
                db.try_lock().is_ok(),
                "root check must not hold the DB lock"
            );
            root_checks.set(root_checks.get() + 1);
            path.is_dir()
        },
        |path| {
            assert!(
                db.try_lock().is_ok(),
                "file check must not hold the DB lock"
            );
            path.is_file()
        },
    );
    assert_eq!(root_checks.get(), 1, "root checks are deduplicated");
    assert_eq!(missing, vec![1, 2]);
}

#[test]
fn manual_output_directory_overrides_the_profile() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let photo_path = dir
        .path()
        .join("photos/VRChat_2026-10-21_21-24-00.000_4x3.png");
    write_photo(&photo_path);
    daemon.run_once().unwrap();

    let export_dir = dir.path().join("exports");
    let job_id = {
        let db = daemon.db.lock().unwrap();
        let photo = db
            .photo_by_path(&photo_path.to_string_lossy())
            .unwrap()
            .expect("indexed photo");
        let profile = db
            .ensure_default_profiles()
            .unwrap()
            .into_iter()
            .next()
            .expect("default profile");
        db.enqueue_job_with_output_dir(
            photo.id,
            &profile.id,
            "keep",
            Some(export_dir.to_str().unwrap()),
        )
        .unwrap()
    };

    daemon.compress_pending().unwrap();

    let db = daemon.db.lock().unwrap();
    let stored = db
        .list_jobs(1)
        .unwrap()
        .into_iter()
        .find(|stored| stored.id == job_id)
        .unwrap();
    assert_eq!(stored.status, QueueStatus::Completed);
    let output = stored.output_path.expect("output path");
    assert!(Path::new(&output).starts_with(&export_dir));
    assert!(Path::new(&output).is_file());
    assert!(
        photo_path.is_file(),
        "keep must leave the original in place"
    );
}

#[test]
fn finds_latest_log() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("output_log_2026-10-20_10-00-00.txt"), "").unwrap();
    std::fs::write(dir.path().join("output_log_2026-10-21_10-00-00.txt"), "").unwrap();
    std::fs::write(dir.path().join("other.txt"), "").unwrap();
    let latest = latest_log_file(dir.path()).unwrap().unwrap();
    assert_eq!(
        latest.file_name().unwrap(),
        "output_log_2026-10-21_10-00-00.txt"
    );
}

#[test]
fn supervisor_label_detects_systemd_main_process() {
    assert_eq!(
        supervisor_label(
            Some(std::ffi::OsStr::new("invocation-id")),
            Some(std::ffi::OsStr::new("42")),
            42
        ),
        "systemd"
    );
    assert_eq!(
        supervisor_label(
            Some(std::ffi::OsStr::new("invocation-id")),
            Some(std::ffi::OsStr::new("41")),
            42
        ),
        "app"
    );
    assert_eq!(
        supervisor_label(Some(std::ffi::OsStr::new("invocation-id")), None, 42),
        "app"
    );
    assert_eq!(
        supervisor_label(None, Some(std::ffi::OsStr::new("42")), 42),
        "app"
    );
}

#[test]
fn daemon_status_reports_pending_sends() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = manual_test_daemon(&dir);
    daemon.scan_photos().unwrap();
    let photo = {
        let db = daemon.db.lock().unwrap();
        db.list_photos(1, 0, vrc_photon_core::model::PhotoSort::Newest)
            .unwrap()[0]
            .id
    };
    {
        let db = daemon.db.lock().unwrap();
        db.enqueue_send(
            photo,
            vrc_photon_core::model::SendKind::Photo,
            vrc_photon_core::model::SendOrigin::Manual,
            vrc_photon_core::model::SendStatus::Pending,
        )
        .unwrap();
    }
    daemon.refresh_status().unwrap();
    assert_eq!(daemon.status.lock().unwrap().pending_sends, 1);
}

#[test]
fn auto_send_runs_after_linking_and_skips_old_photos() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    let log_dir = dir.path().join("logs");
    std::fs::write(
        log_dir.join("output_log_2026-10-21_21-00-00.txt"),
        concat!(
            "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)\n",
            "2026.10.21 21:23:13 Debug      -  [Behaviour] Joining or Creating Room: Test World\n",
        ),
    )
    .unwrap();
    write_photo(&original_path(&dir));
    daemon.ingest_logs().unwrap();
    daemon.scan_photos().unwrap();

    let (captured, _) = {
        let db = Db::open(dir.path().join("library.db")).unwrap();
        let photo = db
            .photo_by_path(&original_path(&dir).to_string_lossy())
            .unwrap()
            .unwrap();
        (photo.captured_at_utc, photo.id)
    };
    save_daemon_settings(&daemon, |settings| {
        settings.discord_enabled = true;
        settings.discord_auto_mode = DiscordAutoMode::Immediate;
        settings.discord_auto_since = Some(captured + 1);
    });
    {
        let db = daemon.db.lock().unwrap();
        save_discord_webhook(&db, "https://discord.com/api/webhooks/1/t").unwrap();
    }

    let second = dir
        .path()
        .join("photos/VRChat_2026-10-21_21-25-00.000_4x3.png");
    write_photo(&second);
    daemon.dirty_paths.push(second.clone());
    daemon.index_dirty_paths().unwrap();

    let db = Db::open(dir.path().join("library.db")).unwrap();
    let history = db.send_history(10).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].entry.origin, SendOrigin::Auto);
    assert_eq!(history[0].entry.status, SendStatus::Pending);
    assert_eq!(
        history[0].photo_name.as_deref(),
        Some("VRChat_2026-10-21_21-25-00.000_4x3.png")
    );
    let second_photo = db
        .photo_by_path(&second.to_string_lossy())
        .unwrap()
        .unwrap();
    assert_eq!(
        db.photo_context(second_photo.id)
            .unwrap()
            .unwrap()
            .world_name
            .as_deref(),
        Some("Test World")
    );
}

#[test]
fn auto_send_runs_after_a_full_scan() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    std::fs::write(
        dir.path().join("logs/output_log_2026-10-21_21-00-00.txt"),
        concat!(
            "2026.10.21 21:23:12 Debug      -  [Behaviour] Joining wrld_test:12345~region(jp)\n",
            "2026.10.21 21:23:13 Debug      -  [Behaviour] Joining or Creating Room: Test World\n",
        ),
    )
    .unwrap();
    save_daemon_settings(&daemon, |settings| {
        settings.discord_enabled = true;
        settings.discord_auto_mode = DiscordAutoMode::Immediate;
        settings.discord_auto_since = Some(0);
    });
    {
        let db = daemon.db.lock().unwrap();
        save_discord_webhook(&db, "https://discord.com/api/webhooks/1/t").unwrap();
    }
    write_photo(&original_path(&dir));
    daemon.ingest_logs().unwrap();
    // The first full scan of an empty library is still "initial": auto
    // sends must not be skipped there, because `discord_auto_since`
    // already filters out old photos.
    daemon.scan_photos().unwrap();

    let db = Db::open(dir.path().join("library.db")).unwrap();
    let history = db.send_history(10).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].entry.origin, SendOrigin::Auto);
    assert_eq!(history[0].entry.status, SendStatus::Pending);
    assert_eq!(
        history[0].photo_name.as_deref(),
        Some("VRChat_2026-10-21_21-24-00.000_4x3.png")
    );
}

#[test]
fn auto_send_enqueue_failure_does_not_fail_the_scan() {
    let dir = tempfile::tempdir().unwrap();
    let mut daemon = test_daemon(&dir);
    save_daemon_settings(&daemon, |settings| {
        settings.discord_enabled = true;
        settings.discord_auto_mode = DiscordAutoMode::Immediate;
        settings.discord_auto_since = Some(0);
    });
    {
        let db = daemon.db.lock().unwrap();
        save_discord_webhook(&db, "https://discord.com/api/webhooks/1/t").unwrap();
        db.connection()
            .execute_batch(
                "CREATE TRIGGER fail_auto_send BEFORE INSERT ON send_outbox \
                 BEGIN SELECT RAISE(ABORT, 'send outbox unavailable'); END;",
            )
            .unwrap();
    }
    write_photo(&original_path(&dir));
    let report = daemon
        .scan_photos()
        .expect("an auto-send enqueue error must not fail the scan");
    assert_eq!(report.indexed, 1);
    assert!(
        daemon
            .db
            .lock()
            .unwrap()
            .send_history(10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cgroup_detects_only_the_daemon_unit() {
    assert!(cgroup_has_unit(
        "0::/user.slice/user-1000.slice/user@1000.service/app.slice/vrc-photon-daemon.service\n",
        "vrc-photon-daemon.service"
    ));
    assert!(!cgroup_has_unit(
        "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-gnome-firefox-123.scope\n",
        "vrc-photon-daemon.service"
    ));
    assert!(!cgroup_has_unit(
        "0::/user.slice/user-1000.slice/user@1000.service/app.slice/foo-vrc-photon-daemon.service\n",
        "vrc-photon-daemon.service"
    ));
}

fn insert_unresolved_world_photo(db: &Db) -> (i64, i64) {
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&NewPhoto {
            root_id: root,
            rel_path: "a.png".to_string(),
            abs_path: "/photos/a.png".to_string(),
            file_name: "a.png".to_string(),
            captured_at_utc: 100,
            captured_at_local: "2026-10-21 21:14:03".to_string(),
            tz_offset_sec: 0,
            width: 1,
            height: 1,
            file_size: 1,
            mtime: 100,
            blake3: None,
        })
        .unwrap();
    let world = db.upsert_world("wrld_test", None, None).unwrap();
    db.set_photo_context(&PhotoContext {
        photo_id: photo,
        session_id: None,
        world_id: Some(world),
        world_name: None,
        instance_id: None,
        link_method: "log".to_string(),
        confidence: 0.9,
        author: None,
        linked_at: "2026-11-03T00:00:00+00:00".to_string(),
    })
    .unwrap();
    (world, photo)
}

#[test]
fn start_workers_spawns_and_stops_the_world_name_worker() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(dir.path().to_path_buf()), None).unwrap();
    daemon.settings.world_name_resolution_enabled = false;
    daemon.start_workers();
    assert!(daemon.world_name_handle.is_some());
    daemon.stop_workers();
    assert!(daemon.world_name_handle.is_none());
}

#[test]
fn reload_settings_syncs_the_world_name_switch() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(dir.path().to_path_buf()), None).unwrap();
    daemon
        .world_name_resolution_enabled
        .store(false, Ordering::Relaxed);
    AppSettings {
        world_name_resolution_enabled: true,
        ..daemon.settings.clone()
    }
    .save(&daemon.db.lock().unwrap())
    .unwrap();

    daemon.reload_settings().unwrap();

    assert!(daemon.world_name_resolution_enabled.load(Ordering::Relaxed));
    assert!(daemon.settings.world_name_resolution_enabled);
}

#[test]
fn start_workers_backfills_when_world_name_resolution_is_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(dir.path().to_path_buf()), None).unwrap();
    let (world, _photo) = {
        let db = daemon.db.lock().unwrap();
        insert_unresolved_world_photo(&db)
    };
    AppSettings {
        world_name_resolution_enabled: true,
        ..daemon.settings.clone()
    }
    .save(&daemon.db.lock().unwrap())
    .unwrap();
    daemon.settings.world_name_resolution_enabled = true;
    daemon
        .world_name_resolution_enabled
        .store(true, Ordering::Relaxed);
    // Keep the worker away from the real network while the test runs.
    daemon
        .db
        .lock()
        .unwrap()
        .set_world_name_next_request_at(chrono::Utc::now().timestamp() + 3_600)
        .unwrap();

    daemon.start_workers();

    assert!(
        daemon
            .db
            .lock()
            .unwrap()
            .world_name_lookup(world)
            .unwrap()
            .is_some(),
        "an enabled startup must enqueue unresolved worlds"
    );
    daemon.stop_workers();
}

#[test]
fn enabling_world_name_resolution_enqueues_backfill() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(dir.path().to_path_buf()), None).unwrap();
    let (world, photo) = {
        let db = daemon.db.lock().unwrap();
        insert_unresolved_world_photo(&db)
    };
    daemon.settings.world_name_resolution_enabled = false;
    AppSettings {
        world_name_resolution_enabled: false,
        ..daemon.settings.clone()
    }
    .save(&daemon.db.lock().unwrap())
    .unwrap();

    AppSettings {
        world_name_resolution_enabled: true,
        ..daemon.settings.clone()
    }
    .save(&daemon.db.lock().unwrap())
    .unwrap();
    daemon.refresh_compression_settings();

    assert!(daemon.world_name_resolution_enabled.load(Ordering::Relaxed));
    assert!(
        daemon
            .db
            .lock()
            .unwrap()
            .world_name_lookup(world)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        daemon
            .db
            .lock()
            .unwrap()
            .photo_context(photo)
            .unwrap()
            .unwrap()
            .world_name,
        None
    );
}

#[test]
fn run_once_does_not_start_the_world_name_worker() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("library.db");
    let mut daemon = Daemon::open(&db_path, Some(dir.path().to_path_buf()), None).unwrap();
    daemon.run_once().unwrap();
    assert!(daemon.world_name_handle.is_none());
}
