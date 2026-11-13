use super::photos::*;
use super::*;
use crate::ai::{NewPhotoAi, PluginInstallation};
use crate::model::{
    DeletionReason, DeletionStatus, NewPhoto, PhotoFilter, PhotoPlayer, PhotoStatus,
};
use vrc_photon_ai_protocol::{
    Catalog, ModelState, TagDefinition, TaggingErrorCode, TaggingJobStatus, TaggingStage,
};

fn sample_photo(root_id: i64, path: &str, ts: i64) -> NewPhoto {
    NewPhoto {
        root_id,
        rel_path: path.trim_start_matches('/').to_string(),
        abs_path: path.to_string(),
        file_name: path.rsplit('/').next().unwrap_or(path).to_string(),
        captured_at_utc: ts,
        captured_at_local: "2026-10-21 21:14:03".to_string(),
        tz_offset_sec: 9 * 3600,
        width: 3840,
        height: 2160,
        file_size: 12_400_000,
        mtime: ts,
        blake3: None,
    }
}

fn photo_at(root_id: i64, path: &str, utc: i64, local: &str) -> NewPhoto {
    let mut photo = sample_photo(root_id, path, utc);
    photo.captured_at_local = local.to_string();
    photo
}

fn profile(id: &str, name: &str) -> CompressionProfile {
    CompressionProfile {
        id: id.to_string(),
        name: name.to_string(),
        ..CompressionProfile::default()
    }
}

fn seed_photo(db: &Db) -> i64 {
    let root = db.add_root("/photos", "reference").unwrap();
    db.upsert_photo(&NewPhoto {
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
    .unwrap()
}

fn seed_other_photo(db: &Db) -> i64 {
    let root = db.add_root("/photos-b", "reference").unwrap();
    db.upsert_photo(&NewPhoto {
        root_id: root,
        rel_path: "b.png".to_string(),
        abs_path: "/photos-b/b.png".to_string(),
        file_name: "b.png".to_string(),
        captured_at_utc: 1_700_000_001,
        captured_at_local: "2026-10-21 21:14:04".to_string(),
        tz_offset_sec: 9 * 3600,
        width: 16,
        height: 10,
        file_size: 10,
        mtime: 1_700_000_001,
        blake3: None,
    })
    .unwrap()
}

const AI_PLUGIN: &str = "vrc-photon-autotag";

fn ai_install(plugin_id: &str, variant: &str, version: &str, active: bool) -> PluginInstallation {
    PluginInstallation {
        plugin_id: plugin_id.to_string(),
        variant: variant.to_string(),
        version: version.to_string(),
        protocol_version: 1,
        path: format!("/plugins/{plugin_id}/{variant}/{version}"),
        manifest_json: format!(r#"{{"version":"{version}"}}"#),
        catalog_version: Some("2026-11-03.1".to_string()),
        active,
        installed_at: "2026-11-04T00:00:00+00:00".to_string(),
        updated_at: "2026-11-04T00:00:00+00:00".to_string(),
    }
}

fn tag(key: &str, name: &str, name_en: Option<&str>, default_threshold: f32) -> TagDefinition {
    TagDefinition {
        key: key.to_string(),
        name: name.to_string(),
        name_en: name_en.map(str::to_string),
        description: format!("{name} description"),
        default_threshold,
        group: String::new(),
    }
}

fn ai_catalog(version: &str, sea_threshold: f32, room_threshold: f32) -> Catalog {
    Catalog {
        version: version.to_string(),
        outdoor: vec![
            tag("sea", "海", Some("Sea"), sea_threshold),
            tag("park", "公園", None, 0.5),
        ],
        indoor: vec![tag(
            "japanese_room",
            "和室",
            Some("Japanese room"),
            room_threshold,
        )],
        atmosphere: Vec::new(),
    }
}

fn ai_catalog_with_atmosphere(version: &str) -> Catalog {
    let mut catalog = ai_catalog(version, 0.5, 0.5);
    let mut sunset = tag("sunset", "夕景", Some("Sunset"), 0.5);
    sunset.group = "scene".to_string();
    let mut weather_clear = tag("weather_clear", "晴れ", Some("Clear / sunny"), 0.5);
    weather_clear.group = "weather".to_string();
    catalog.atmosphere = vec![sunset, weather_clear];
    catalog
}

fn seed_ai_tag(
    db: &Db,
    photo_id: i64,
    plugin_id: &str,
    tag_key: &str,
    probability: f32,
    applied: bool,
) {
    db.connection()
            .execute(
                "INSERT INTO photo_ai_tags(photo_id, plugin_id, tag_key, probability, applied, \
                    model_version, catalog_version, tagged_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 'd1-2B-Q8_0', '2026-11-03.1', '2026-11-04T00:00:00+00:00')",
                params![photo_id, plugin_id, tag_key, probability, i64::from(applied)],
            )
            .unwrap();
}

fn photo_to_new(photo: &Photo) -> NewPhoto {
    NewPhoto {
        root_id: photo.root_id,
        rel_path: photo.rel_path.clone(),
        abs_path: photo.abs_path.clone(),
        file_name: photo.file_name.clone(),
        captured_at_utc: photo.captured_at_utc,
        captured_at_local: photo.captured_at_local.clone(),
        tz_offset_sec: photo.tz_offset_sec,
        width: photo.width,
        height: photo.height,
        file_size: photo.file_size,
        mtime: photo.mtime,
        blake3: photo.blake3.clone(),
    }
}

fn insert_deletion_request(db: &Db, photo_id: i64, reason: &str, status: &str) {
    db.connection()
        .execute(
            "INSERT INTO deletion_requests(photo_id, reason, status, requested_at) \
                 VALUES (?1, ?2, ?3, ?4)",
            params![photo_id, reason, status, "2026-10-29T00:00:00+00:00"],
        )
        .unwrap();
}

#[test]
fn concurrent_open_of_a_fresh_database_succeeds() {
    // UI and daemon opening the same new database at once must both
    // succeed: the immediate transactions serialize the migrations.
    for round in 0..3 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("photos-{round}.db"));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let barrier = barrier.clone();
            let path = path.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                Db::open(&path).map(|db| {
                    db.conn
                        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                })
            }));
        }
        for handle in handles {
            let opened = handle.join().unwrap();
            assert_eq!(opened.unwrap().unwrap(), SCHEMA_VERSION);
        }
    }
}

#[test]
fn deleting_photo_is_hidden_from_index_search_and_counts() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.add_derivative(id, "/photos/a.webp", "webp", 16, 10, 8, Some("p"), CompressionOrigin::Manual, "keep")
        .unwrap();
    db.connection()
        .execute(
            "UPDATE photos SET status = 'deleting' WHERE id = ?1",
            params![id],
        )
        .unwrap();
    assert!(db.photo_index(PhotoSort::Newest).unwrap().is_empty());
    assert_eq!(db.count_photos().unwrap(), 0);
    let stats = db.stats().unwrap();
    assert_eq!(stats.photo_count, 0);
    assert_eq!(stats.compressed_count, 0);
    assert_eq!(stats.derivative_bytes, 0);
    assert!(db.compressed_photo_ids().unwrap().is_empty());
    assert!(
        db.search_photos(&PhotoFilter::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn rebind_candidate_skips_photos_with_active_deletion() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.set_photo_blake3(id, "hash").unwrap();
    let photo = db.photo_by_id(id).unwrap().unwrap();
    db.request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap();
    let mut moved = photo_to_new(&photo);
    moved.abs_path = "/photos/moved.png".to_string();
    moved.rel_path = "moved.png".to_string();
    moved.blake3 = Some("hash".to_string());
    assert!(db.find_rebind_candidate(&moved).unwrap().is_none());
    assert!(!db.rebind_photo(id, &moved).unwrap());
}

#[test]
fn upsert_preserves_deleting_status_while_a_request_is_active() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    insert_deletion_request(&db, id, "manual", "pending");
    db.connection()
        .execute(
            "UPDATE photos SET status = 'deleting' WHERE id = ?1",
            params![id],
        )
        .unwrap();
    let photo = db.photo_by_id(id).unwrap().unwrap();
    db.upsert_photo(&photo_to_new(&photo)).unwrap();
    assert_eq!(
        db.photo_by_id(id).unwrap().unwrap().status,
        PhotoStatus::Deleting
    );
    assert_eq!(db.count_photos().unwrap(), 0);
}

#[test]
fn upsert_preserves_replaced_display_while_auto_deletion_is_active() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    let derivative = db
        .add_derivative(id, "/photos/a_webp.webp", "webp", 16, 10, 8, Some("p"), CompressionOrigin::Manual, "replace")
        .unwrap();
    db.set_replaced(id, derivative).unwrap();
    insert_deletion_request(&db, id, "auto_replace", "pending");
    let photo = db.photo_by_id(id).unwrap().unwrap();
    db.upsert_photo(&photo_to_new(&photo)).unwrap();
    let photo = db.photo_by_id(id).unwrap().unwrap();
    assert_eq!(photo.status, PhotoStatus::Replaced);
    assert_eq!(photo.display_derivative_id, Some(derivative));
}

#[test]
fn set_replaced_does_not_unhide_a_deleting_photo() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    let derivative = db
        .add_derivative(id, "/photos/a_webp.webp", "webp", 16, 10, 8, Some("p"), CompressionOrigin::Manual, "replace")
        .unwrap();
    insert_deletion_request(&db, id, "manual", "pending");
    db.connection()
        .execute(
            "UPDATE photos SET status = 'deleting' WHERE id = ?1",
            params![id],
        )
        .unwrap();
    assert!(db.set_replaced(id, derivative).unwrap());
    let photo = db.photo_by_id(id).unwrap().unwrap();
    assert_eq!(photo.status, PhotoStatus::Deleting);
    assert_eq!(photo.display_derivative_id, Some(derivative));
    assert_eq!(db.count_photos().unwrap(), 0);
}

#[test]
fn complete_replacement_marks_replaced_and_requests_auto_deletion() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    let derivative = db
        .add_derivative(id, "/photos/a_webp.webp", "webp", 16, 10, 8, Some("p"), CompressionOrigin::Manual, "replace")
        .unwrap();

    db.complete_replacement(id, derivative).unwrap();

    let photo = db.photo_by_id(id).unwrap().unwrap();
    assert_eq!(photo.status, PhotoStatus::Replaced);
    assert_eq!(photo.display_derivative_id, Some(derivative));
    let request = db.active_deletion_request(id).unwrap().unwrap();
    assert_eq!(request.reason, DeletionReason::AutoReplace);
    assert_eq!(request.status, DeletionStatus::Pending);
}

#[test]
fn complete_replacement_rolls_back_when_derivative_is_missing() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);

    assert!(db.complete_replacement(id, 9999).is_err());

    let photo = db.photo_by_id(id).unwrap().unwrap();
    assert_eq!(photo.status, PhotoStatus::Present);
    assert!(photo.display_derivative_id.is_none());
    assert!(db.active_deletion_request(id).unwrap().is_none());
}

#[test]
fn manual_deletion_request_hides_photo_and_is_idempotent() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    let first = db
        .request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap();
    let second = db
        .request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(
        db.photo_by_id(id).unwrap().unwrap().status,
        PhotoStatus::Deleting
    );
    let request = db.active_deletion_request(id).unwrap().unwrap();
    assert_eq!(request.reason, DeletionReason::Manual);
    assert_eq!(request.status, DeletionStatus::Pending);
}

#[test]
fn photo_effective_path_prefers_the_display_derivative() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    assert_eq!(
        db.photo_effective_path(id).unwrap().as_deref(),
        Some("/photos/a.png")
    );
    assert_eq!(db.photo_effective_path(999_999).unwrap(), None);

    let derivative = db
        .add_derivative(id, "/photos/a_webp.webp", "webp", 16, 10, 8, Some("p"), CompressionOrigin::Manual, "replace")
        .unwrap();
    db.set_replaced(id, derivative).unwrap();

    assert_eq!(
        db.photo_effective_path(id).unwrap().as_deref(),
        Some("/photos/a_webp.webp")
    );
}

#[test]
fn auto_replace_request_keeps_the_photo_visible() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.request_deletion(&[id], DeletionReason::AutoReplace, false)
        .unwrap();
    assert_eq!(
        db.photo_by_id(id).unwrap().unwrap().status,
        PhotoStatus::Present
    );
    assert_eq!(db.count_photos().unwrap(), 1);
}

#[test]
fn manual_request_upgrades_an_active_auto_request() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.request_deletion(&[id], DeletionReason::AutoReplace, false)
        .unwrap();
    db.request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap();
    let request = db.active_deletion_request(id).unwrap().unwrap();
    assert_eq!(request.reason, DeletionReason::Manual);
    assert_eq!(
        db.photo_by_id(id).unwrap().unwrap().status,
        PhotoStatus::Deleting
    );
}

#[test]
fn unknown_deletion_reason_is_preserved_and_cannot_be_requested() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.connection()
        .execute(
            "INSERT INTO deletion_requests(photo_id, reason, status, force, requested_at) \
                 VALUES (?1, 'future_reason', 'pending', 0, ?2)",
            params![id, now_string()],
        )
        .unwrap();

    let pending = db.pending_deletion_requests(10).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].reason, DeletionReason::Unknown);
    assert_ne!(pending[0].reason, DeletionReason::Manual);
    assert!(
        db.request_deletion(&[id], DeletionReason::Unknown, false)
            .is_err()
    );
}

#[test]
fn enqueue_rejects_photos_with_active_deletion() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.save_profile(&profile("p", "P")).unwrap();
    db.request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap();
    assert!(
        !db.enqueue_job_for_existing_profile(id, "p", "keep")
            .unwrap()
    );
    assert!(db.enqueue_job(id, "p", "keep").is_err());
    assert_eq!(db.count_active_jobs().unwrap(), 0);
}

#[test]
fn deletion_job_summary_counts_each_status() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.save_profile(&profile("p", "P")).unwrap();
    db.enqueue_job(id, "p", "keep").unwrap();
    let waiting = db.enqueue_job(id, "p", "keep").unwrap();
    db.update_job(&waiting, QueueStatus::Waiting, 0.0, None, None)
        .unwrap();
    let failed = db.enqueue_job(id, "p", "keep").unwrap();
    db.update_job(&failed, QueueStatus::Failed, 0.0, Some("boom"), None)
        .unwrap();
    let summary = db.deletion_job_summary(&[id]).unwrap();
    assert_eq!(summary.compression.pending, 1);
    assert_eq!(summary.compression.waiting, 1);
    assert_eq!(summary.compression.failed, 1);
    assert_eq!(summary.send.total(), 0);
    assert_eq!(summary.affected_photos, 1);
    assert!(summary.needs_warning());

    let other = seed_other_photo(&db);
    let summary = db.deletion_job_summary(&[id, other]).unwrap();
    assert_eq!(
        summary.affected_photos, 1,
        "photos without jobs are excluded"
    );
}

#[test]
fn deletion_job_summary_keeps_unknown_status_distinct() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.save_profile(&profile("p", "P")).unwrap();
    let job = db.enqueue_job(id, "p", "keep").unwrap();
    db.connection()
        .execute(
            "UPDATE compression_jobs SET status = 'future_status' WHERE id = ?1",
            params![job],
        )
        .unwrap();

    let summary = db.deletion_job_summary(&[id]).unwrap();
    assert_eq!(
        summary.compression.pending, 0,
        "unknown statuses must not be treated as pending"
    );
    assert_eq!(
        summary.compression.unknown, 1,
        "unknown statuses keep their own count"
    );
    assert_eq!(
        summary.affected_photos, 1,
        "the heading must not disagree with the body"
    );
}

#[test]
fn deletion_blockers_count_active_sends_but_not_failed_sends() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let id = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    assert_eq!(db.deletion_blockers(photo).unwrap().active, 1);
    db.claim_next_send(100).unwrap();
    db.fail_send(id, SendErrorCode::Network, None, None, None, None)
        .unwrap();
    assert_eq!(db.deletion_blockers(photo).unwrap().active, 0);
}

#[test]
fn deletion_summary_includes_send_statuses() {
    let db = Db::open_in_memory().unwrap();
    let waiting_photo = seed_photo(&db);
    let pending_photo = seed_other_photo(&db);
    db.enqueue_send(
        waiting_photo,
        SendKind::Photo,
        SendOrigin::Auto,
        SendStatus::Waiting,
    )
    .unwrap();
    db.enqueue_send(
        pending_photo,
        SendKind::Photo,
        SendOrigin::Manual,
        SendStatus::Pending,
    )
    .unwrap();
    let summary = db
        .deletion_job_summary(&[waiting_photo, pending_photo])
        .unwrap();
    assert_eq!(summary.send.waiting, 1);
    assert_eq!(summary.send.pending, 1);
    assert_eq!(
        summary.affected_photos, 2,
        "both photos have a pending send"
    );
}

#[test]
fn deletion_summary_counts_a_photo_with_jobs_and_sends_once() {
    let db = Db::open_in_memory().unwrap();
    let both = seed_photo(&db);
    let send_only = seed_other_photo(&db);

    // Create and fail the send-only photo's row first so the claim below
    // cannot pick the combined photo's send.
    let failed = db
        .enqueue_send(
            send_only,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.claim_next_send(100).unwrap();
    db.fail_send(failed, SendErrorCode::Network, None, None, None, None)
        .unwrap();

    db.save_profile(&profile("p", "P")).unwrap();
    db.enqueue_job(both, "p", "keep").unwrap();
    db.enqueue_send(
        both,
        SendKind::Photo,
        SendOrigin::Manual,
        SendStatus::Pending,
    )
    .unwrap();

    let summary = db.deletion_job_summary(&[both, send_only]).unwrap();
    assert_eq!(summary.compression.pending, 1);
    assert_eq!(summary.send.pending, 1);
    assert_eq!(summary.send.failed, 1);
    assert_eq!(
        summary.affected_photos, 2,
        "a photo with both kinds of work counts once, and a failed send alone counts"
    );
}

#[test]
fn deletion_summary_counts_a_processing_send_as_affected() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    db.enqueue_send(
        photo,
        SendKind::Photo,
        SendOrigin::Manual,
        SendStatus::Pending,
    )
    .unwrap();
    let claimed = db.claim_next_send(100).unwrap().unwrap();
    assert_eq!(claimed.status, SendStatus::Processing);

    let summary = db.deletion_job_summary(&[photo]).unwrap();
    assert_eq!(summary.send.processing, 1);
    assert_eq!(summary.send.total(), 1);
    assert_eq!(
        summary.affected_photos, 1,
        "a photo with a claimed send is affected"
    );
    assert!(summary.needs_warning());
}

#[test]
fn deletion_summary_excludes_photos_with_only_terminal_sends() {
    let db = Db::open_in_memory().unwrap();
    let sent_photo = seed_photo(&db);
    let cancelled_photo = seed_other_photo(&db);

    let sent = db
        .enqueue_send(
            sent_photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.claim_next_send(100).unwrap();
    db.complete_send(
        sent,
        &SendCompletion {
            source: None,
            profile_id: None,
            destination: "D".into(),
            message_id: None,
            channel_id: None,
            guild_id: None,
        },
    )
    .unwrap();

    let cancelled = db
        .enqueue_send(
            cancelled_photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    assert!(db.cancel_send(cancelled).unwrap());

    let summary = db
        .deletion_job_summary(&[sent_photo, cancelled_photo])
        .unwrap();
    assert_eq!(summary.send.total(), 0);
    assert_eq!(
        summary.affected_photos, 0,
        "sent and cancelled sends are terminal and leave the photo unaffected"
    );
    assert!(!summary.needs_warning());
}

#[test]
fn deletion_summary_counts_an_unknown_send_status_as_affected() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let send = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.connection()
        .execute(
            "UPDATE send_outbox SET status = 'future_status' WHERE id = ?1",
            params![send],
        )
        .unwrap();

    let summary = db.deletion_job_summary(&[photo]).unwrap();
    assert_eq!(
        summary.send.pending, 1,
        "unknown send statuses parse as pending in the body"
    );
    assert_eq!(
        summary.affected_photos, 1,
        "a status written by a newer version is never terminal"
    );
}

#[test]
fn pending_deletion_requests_skip_requests_with_active_jobs() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let blocked = db
        .upsert_photo(&sample_photo(root, "/photos/blocked.png", 1_700_000_000))
        .unwrap();
    let ready = db
        .upsert_photo(&sample_photo(root, "/photos/ready.png", 1_700_000_001))
        .unwrap();
    let unknown = db
        .upsert_photo(&sample_photo(root, "/photos/unknown.png", 1_700_000_002))
        .unwrap();
    db.save_profile(&profile("p", "P")).unwrap();

    db.enqueue_job(blocked, "p", "keep").unwrap();
    db.enqueue_job(unknown, "p", "keep").unwrap();
    let blocked_request = db
        .request_deletion(&[blocked], DeletionReason::Manual, false)
        .unwrap()[0];
    let ready_request = db
        .request_deletion(&[ready], DeletionReason::Manual, false)
        .unwrap()[0];
    db.connection()
        .execute(
            "INSERT INTO deletion_requests(photo_id, reason, status, force, requested_at) \
                 VALUES (?1, 'future_reason', 'pending', 0, ?2)",
            params![unknown, now_string()],
        )
        .unwrap();

    let pending = db.pending_deletion_requests(10).unwrap();
    assert_eq!(pending.len(), 2);
    assert!(pending.iter().any(|request| request.id == ready_request));
    assert!(
        pending
            .iter()
            .any(|request| request.reason == DeletionReason::Unknown),
        "unknown reasons must be returned even while jobs are active"
    );
    assert!(!pending.iter().any(|request| request.id == blocked_request));
}

#[test]
fn pending_deletion_requests_skip_active_sends_and_ignore_test_rows() {
    let db = Db::open_in_memory().unwrap();
    let blocked = seed_photo(&db);
    let ready = seed_other_photo(&db);
    db.enqueue_test_send().unwrap();
    // auto_replace keeps the photo visible so a send can be queued after
    // the request is recorded.
    let blocked_request = db
        .request_deletion(&[blocked], DeletionReason::AutoReplace, false)
        .unwrap()[0];
    let ready_request = db
        .request_deletion(&[ready], DeletionReason::AutoReplace, false)
        .unwrap()[0];
    db.enqueue_send(
        blocked,
        SendKind::Photo,
        SendOrigin::Manual,
        SendStatus::Pending,
    )
    .unwrap();

    let pending = db.pending_deletion_requests(10).unwrap();
    assert!(pending.iter().any(|request| request.id == ready_request));
    assert!(
        !pending.iter().any(|request| request.id == blocked_request),
        "an active send must hide the request"
    );
}

#[test]
fn pending_deletion_requests_prefer_manual_over_older_backlog() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let auto_old = db
        .upsert_photo(&sample_photo(root, "/photos/auto-old.png", 1_700_000_000))
        .unwrap();
    let auto_new = db
        .upsert_photo(&sample_photo(root, "/photos/auto-new.png", 1_700_000_001))
        .unwrap();
    let manual_earlier = db
        .upsert_photo(&sample_photo(
            root,
            "/photos/manual-earlier.png",
            1_700_000_002,
        ))
        .unwrap();
    let manual_later = db
        .upsert_photo(&sample_photo(
            root,
            "/photos/manual-later.png",
            1_700_000_003,
        ))
        .unwrap();

    let auto_old_request = db
        .request_deletion(&[auto_old], DeletionReason::AutoReplace, false)
        .unwrap()[0];
    let auto_new_request = db
        .request_deletion(&[auto_new], DeletionReason::AutoReplace, false)
        .unwrap()[0];
    let manual_earlier_request = db
        .request_deletion(&[manual_earlier], DeletionReason::Manual, false)
        .unwrap()[0];
    let manual_later_request = db
        .request_deletion(&[manual_later], DeletionReason::Manual, false)
        .unwrap()[0];

    for (request, requested_at) in [
        (auto_old_request, "2026-10-21T00:00:01Z"),
        (auto_new_request, "2026-10-21T00:00:02Z"),
        (manual_earlier_request, "2026-10-21T00:00:03Z"),
        (manual_later_request, "2026-10-21T00:00:04Z"),
    ] {
        db.connection()
            .execute(
                "UPDATE deletion_requests SET requested_at = ?1 WHERE id = ?2",
                params![requested_at, request],
            )
            .unwrap();
    }

    // A manual request must enter the row window even when older backlog
    // already fills it.
    let window = db.pending_deletion_requests(2).unwrap();
    let ids = |requests: &[DeletionRequest]| {
        requests
            .iter()
            .map(|request| request.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(&window),
        vec![manual_earlier_request, manual_later_request],
        "manual requests must preempt an older auto_replace backlog"
    );

    // Both groups keep requested_at order.
    let pending = db.pending_deletion_requests(4).unwrap();
    assert_eq!(
        ids(&pending),
        vec![
            manual_earlier_request,
            manual_later_request,
            auto_old_request,
            auto_new_request
        ]
    );
}

#[test]
fn claim_deletion_requires_no_active_jobs() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.save_profile(&profile("p", "P")).unwrap();
    db.enqueue_job(id, "p", "keep").unwrap();
    let request = db
        .request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap()[0];
    assert!(db.claim_deletion(request).unwrap().is_none());
    let job = db.next_pending_job().unwrap().unwrap();
    db.update_job(&job.id, QueueStatus::Completed, 1.0, None, None)
        .unwrap();
    assert!(db.claim_deletion(request).unwrap().is_some());
    assert_eq!(
        db.active_deletion_request(id).unwrap().unwrap().status,
        DeletionStatus::Running
    );
}

#[test]
fn claim_deletion_blocks_failed_jobs_unless_forced() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.save_profile(&profile("p", "P")).unwrap();
    let job = db.enqueue_job(id, "p", "keep").unwrap();
    db.update_job(&job, QueueStatus::Failed, 0.0, Some("boom"), None)
        .unwrap();
    let request = db
        .request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap()[0];
    assert!(db.claim_deletion(request).unwrap().is_none());
    assert!(db.force_deletion(request).unwrap());
    assert!(db.claim_deletion(request).unwrap().is_some());
}

#[test]
fn claim_deletion_will_not_start_while_a_send_is_processing() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    // auto_replace は写真を deleting にしないため、要求後に送信を積める。
    let request = db
        .request_deletion(&[photo], DeletionReason::AutoReplace, false)
        .unwrap()[0];
    db.enqueue_send(
        photo,
        SendKind::Photo,
        SendOrigin::Manual,
        SendStatus::Pending,
    )
    .unwrap();
    db.claim_next_send(100).unwrap();
    assert!(db.claim_deletion(request).unwrap().is_none());
}

#[test]
fn claim_deletion_starts_after_an_in_flight_send_reaches_a_terminal_state() {
    let db = Db::open_in_memory().unwrap();
    let failed_photo = seed_photo(&db);
    let cancelled_photo = seed_other_photo(&db);
    // auto_replace は写真を deleting にしないため、要求後に送信を積める。
    let failed_request = db
        .request_deletion(&[failed_photo], DeletionReason::AutoReplace, false)
        .unwrap()[0];
    let cancelled_request = db
        .request_deletion(&[cancelled_photo], DeletionReason::AutoReplace, false)
        .unwrap()[0];
    let failed_send = db
        .enqueue_send(
            failed_photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    let cancelled_send = db
        .enqueue_send(
            cancelled_photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.claim_next_send(100).unwrap();
    assert!(db.claim_deletion(failed_request).unwrap().is_none());
    assert!(db.claim_deletion(cancelled_request).unwrap().is_none());

    db.fail_send(failed_send, SendErrorCode::Http401, None, None, None, None)
        .unwrap();
    assert!(db.claim_deletion(failed_request).unwrap().is_some());

    assert!(db.cancel_send(cancelled_send).unwrap());
    assert!(db.claim_deletion(cancelled_request).unwrap().is_some());
}

#[test]
fn force_deletion_cancels_queued_but_not_processing_jobs() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.save_profile(&profile("p", "P")).unwrap();
    let processing = db.enqueue_job(id, "p", "keep").unwrap();
    let pending = db.enqueue_job(id, "p", "keep").unwrap();
    let started = db.next_pending_job().unwrap().unwrap();
    assert_eq!(started.id, processing);
    let request = db
        .request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap()[0];
    db.force_deletion(request).unwrap();
    let jobs = db.list_jobs(10).unwrap();
    let by_id = |id: &str| jobs.iter().find(|job| job.id == id).unwrap();
    assert_eq!(by_id(&pending).status, QueueStatus::Cancelled);
    assert_eq!(by_id(&processing).status, QueueStatus::Processing);
    assert!(
        db.claim_deletion(request).unwrap().is_none(),
        "the processing job must block the forced claim"
    );
    db.update_job(&processing, QueueStatus::Completed, 1.0, None, None)
        .unwrap();
    assert!(db.claim_deletion(request).unwrap().is_some());
}

#[test]
fn manual_deletion_cancels_waiting_pending_and_failed_sends() {
    let db = Db::open_in_memory().unwrap();
    let waiting_photo = seed_photo(&db);
    let pending_photo = seed_other_photo(&db);
    let failed_root = db.add_root("/photos-c", "reference").unwrap();
    let failed_photo = db
        .upsert_photo(&photo_at(
            failed_root,
            "c.png",
            1_700_000_002,
            "2026-10-21 21:14:05",
        ))
        .unwrap();
    // 最初に failed 行を作って claim し、processing から failed にする。
    let failed = db
        .enqueue_send(
            failed_photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.claim_next_send(100).unwrap();
    db.fail_send(failed, SendErrorCode::Http401, None, None, None, None)
        .unwrap();
    // その後で待機・実行待ちを積む。
    let waiting = db
        .enqueue_send(
            waiting_photo,
            SendKind::Photo,
            SendOrigin::Auto,
            SendStatus::Waiting,
        )
        .unwrap()
        .unwrap();
    let pending = db
        .enqueue_send(
            pending_photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.request_deletion(
        &[waiting_photo, pending_photo, failed_photo],
        DeletionReason::Manual,
        false,
    )
    .unwrap();
    assert_eq!(
        db.send_entry_by_id(waiting).unwrap().unwrap().status,
        SendStatus::Cancelled
    );
    assert_eq!(
        db.send_entry_by_id(pending).unwrap().unwrap().status,
        SendStatus::Cancelled
    );
    assert_eq!(
        db.send_entry_by_id(failed).unwrap().unwrap().status,
        SendStatus::Cancelled
    );
}

#[test]
fn force_deletion_leaves_a_processing_send_running() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let request = db
        .request_deletion(&[photo], DeletionReason::AutoReplace, false)
        .unwrap()[0];
    let id = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.claim_next_send(100).unwrap();
    db.force_deletion(request).unwrap();
    assert_eq!(
        db.send_entry_by_id(id).unwrap().unwrap().status,
        SendStatus::Processing
    );
    assert!(db.claim_deletion(request).unwrap().is_none());
}

#[test]
fn cancel_deletion_restores_visible_status_until_running() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    let request = db
        .request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap()[0];
    assert!(db.cancel_deletion(request).unwrap());
    assert_eq!(
        db.photo_by_id(id).unwrap().unwrap().status,
        PhotoStatus::Present
    );
    assert!(db.active_deletion_request(id).unwrap().is_none());
    let second = db
        .request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap()[0];
    assert!(db.claim_deletion(second).unwrap().is_some());
    assert!(!db.cancel_deletion(second).unwrap());
}

#[test]
fn cancelling_a_deletion_does_not_restore_cancelled_sends() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let send = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    let request = db
        .request_deletion(&[photo], DeletionReason::Manual, false)
        .unwrap()[0];
    db.cancel_deletion(request).unwrap();
    assert_eq!(
        db.send_entry_by_id(send).unwrap().unwrap().status,
        SendStatus::Cancelled
    );
}

#[test]
fn claim_deletion_returns_the_upgraded_request() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    let request = db
        .request_deletion(&[id], DeletionReason::AutoReplace, false)
        .unwrap()[0];
    db.request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap();
    let claimed = db.claim_deletion(request).unwrap().unwrap();
    assert_eq!(claimed.reason, DeletionReason::Manual);
    assert_eq!(claimed.status, DeletionStatus::Running);
}

#[test]
fn block_job_failed_uses_the_current_force_flag() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.save_profile(&profile("p", "P")).unwrap();
    let job = db.enqueue_job(id, "p", "keep").unwrap();
    db.update_job(&job, QueueStatus::Failed, 0.0, Some("boom"), None)
        .unwrap();
    let request = db
        .request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap()[0];
    assert!(db.block_deletion_if_jobs_failed(request).unwrap());
    assert_eq!(
        db.active_deletion_request(id).unwrap().unwrap().status,
        DeletionStatus::Blocked
    );
    assert!(db.force_deletion(request).unwrap());
    assert!(!db.block_deletion_if_jobs_failed(request).unwrap());
    assert!(db.claim_deletion(request).unwrap().is_some());
}

#[test]
fn block_unknown_blocks_a_still_unknown_reason() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.connection()
        .execute(
            "INSERT INTO deletion_requests(photo_id, reason, status, force, requested_at) \
                 VALUES (?1, 'future_reason', 'pending', 0, '2026-10-21T00:00:00Z')",
            params![id],
        )
        .unwrap();
    let request = db.active_deletion_request(id).unwrap().unwrap();
    assert_eq!(request.reason, DeletionReason::Unknown);
    assert!(db.block_deletion_if_unknown(request.id).unwrap());
    let blocked = db.active_deletion_request(id).unwrap().unwrap();
    assert_eq!(blocked.status, DeletionStatus::Blocked);
    assert_eq!(blocked.blocked_reason.as_deref(), Some("unknown_reason"));
}

#[test]
fn block_unknown_skips_a_request_upgraded_to_manual() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.connection()
        .execute(
            "INSERT INTO deletion_requests(photo_id, reason, status, force, requested_at) \
                 VALUES (?1, 'future_reason', 'pending', 0, '2026-10-21T00:00:00Z')",
            params![id],
        )
        .unwrap();
    let request = db.active_deletion_request(id).unwrap().unwrap();
    // The UI upgrades the same request to manual after the executor's
    // snapshot was taken; the conditional block must not touch it.
    db.request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap();
    assert!(!db.block_deletion_if_unknown(request.id).unwrap());
    let current = db.active_deletion_request(id).unwrap().unwrap();
    assert_eq!(current.reason, DeletionReason::Manual);
    assert_eq!(current.status, DeletionStatus::Pending);
    assert!(db.claim_deletion(request.id).unwrap().is_some());
}

#[test]
fn reset_running_deletions_makes_requests_retryable() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    let request = db
        .request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap()[0];
    assert!(db.claim_deletion(request).unwrap().is_some());
    assert_eq!(db.reset_running_deletions().unwrap(), 1);
    let reset = db.active_deletion_request(id).unwrap().unwrap();
    assert_eq!(reset.status, DeletionStatus::Pending);
    assert!(
        reset.started_at.is_some(),
        "restart must keep the partial-execution marker"
    );
    assert!(
        !db.cancel_deletion(request).unwrap(),
        "cancel must refuse after a started deletion"
    );
}

#[test]
fn cancel_deletion_rejects_auto_replace_requests() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    let request = db
        .request_deletion(&[id], DeletionReason::AutoReplace, false)
        .unwrap()[0];
    assert!(!db.cancel_deletion(request).unwrap());
    assert_eq!(
        db.active_deletion_request(id).unwrap().unwrap().status,
        DeletionStatus::Pending
    );
}

#[test]
fn manual_request_is_rejected_while_a_request_is_running() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    let request = db
        .request_deletion(&[id], DeletionReason::AutoReplace, false)
        .unwrap()[0];
    assert!(db.claim_deletion(request).unwrap().is_some());
    let error = db
        .request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap_err();
    assert!(error.to_string().contains("running"));
    assert_eq!(
        db.active_deletion_request(id).unwrap().unwrap().reason,
        DeletionReason::AutoReplace
    );
}

#[test]
fn cancel_deletion_refuses_after_execution_started() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    let request = db
        .request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap()[0];
    assert!(db.claim_deletion(request).unwrap().is_some());
    db.block_deletion(request, "file_error", Some("permission denied"))
        .unwrap();
    assert!(!db.cancel_deletion(request).unwrap());
    assert_eq!(
        db.active_deletion_request(id).unwrap().unwrap().status,
        DeletionStatus::Blocked
    );
}

#[test]
fn resume_blocked_deletion_waits_for_all_jobs() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.save_profile(&profile("p", "P")).unwrap();
    let failed = db.enqueue_job(id, "p", "keep").unwrap();
    db.update_job(&failed, QueueStatus::Failed, 0.0, Some("boom"), None)
        .unwrap();
    let request = db
        .request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap()[0];
    db.block_deletion(request, "job_failed", None).unwrap();
    assert_eq!(db.resume_blocked_deletions_for_photo(id).unwrap(), 0);

    db.retry_job(&failed).unwrap();
    let retried = db.next_pending_job().unwrap().unwrap();
    db.update_job(&retried.id, QueueStatus::Completed, 1.0, None, None)
        .unwrap();
    assert_eq!(db.resume_blocked_deletions_for_photo(id).unwrap(), 1);
    assert_eq!(
        db.active_deletion_request(id).unwrap().unwrap().status,
        DeletionStatus::Pending
    );
}

#[test]
fn retry_is_rejected_while_deletion_is_running() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.save_profile(&profile("p", "P")).unwrap();
    let failed = db.enqueue_job(id, "p", "keep").unwrap();
    db.update_job(&failed, QueueStatus::Failed, 0.0, Some("boom"), None)
        .unwrap();
    let request = db
        .request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap()[0];
    db.force_deletion(request).unwrap();
    assert!(db.claim_deletion(request).unwrap().is_some());
    assert!(!db.retry_job(&failed).unwrap());
    assert_eq!(db.retry_failed_jobs().unwrap(), 0);
    assert_eq!(
        db.list_jobs(10)
            .unwrap()
            .into_iter()
            .find(|job| job.id == failed)
            .unwrap()
            .status,
        QueueStatus::Failed
    );
}

#[test]
fn photo_index_orders_newest_first_and_uses_local_month() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let older = db
        .upsert_photo(&photo_at(
            root,
            "/photos/older.png",
            1_700_000_000,
            "2025-04-30 09:00:00",
        ))
        .unwrap();
    let newer = db
        .upsert_photo(&photo_at(
            root,
            "/photos/newer.png",
            1_700_100_000,
            "2025-05-01 08:00:00",
        ))
        .unwrap();
    let missing = db
        .upsert_photo(&photo_at(
            root,
            "/photos/missing.png",
            1_700_200_000,
            "2025-06-01 08:00:00",
        ))
        .unwrap();
    db.mark_missing(missing).unwrap();

    let entries = db.photo_index(PhotoSort::Newest).unwrap();
    assert_eq!(
        entries.iter().map(|entry| entry.id).collect::<Vec<_>>(),
        vec![newer, older]
    );
    assert_eq!(entries[0].captured_month, 202505);
    assert_eq!(entries[1].captured_month, 202504);
}

#[test]
fn photo_index_snapshot_limits_rows_and_reports_truncation() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    for (index, path) in ["one.png", "two.png", "three.png"].iter().enumerate() {
        db.upsert_photo(&photo_at(
            root,
            &format!("/photos/{path}"),
            1_700_000_000 + index as i64,
            "2025-05-01 08:00:00",
        ))
        .unwrap();
    }

    let snapshot = db
        .photo_index_snapshot_with_limit(PhotoSort::Newest, 2)
        .unwrap();

    assert_eq!(snapshot.entries.len(), 2);
    assert!(snapshot.truncated);
}

#[test]
fn photo_index_snapshot_does_not_truncate_at_or_below_the_limit() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    for (index, path) in ["one.png", "two.png"].iter().enumerate() {
        db.upsert_photo(&photo_at(
            root,
            &format!("/photos/{path}"),
            1_700_000_000 + index as i64,
            "2025-05-01 08:00:00",
        ))
        .unwrap();
    }

    let snapshot = db
        .photo_index_snapshot_with_limit(PhotoSort::Newest, 2)
        .unwrap();

    assert_eq!(snapshot.entries.len(), 2);
    assert!(!snapshot.truncated);
}

#[test]
fn photo_index_supports_world_sort() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let alpha = db
        .upsert_photo(&photo_at(
            root,
            "/photos/alpha.png",
            1_700_000_000,
            "2025-04-30 09:00:00",
        ))
        .unwrap();
    let beta = db
        .upsert_photo(&photo_at(
            root,
            "/photos/beta.png",
            1_700_000_100,
            "2025-04-30 10:00:00",
        ))
        .unwrap();
    for (photo_id, world_name) in [(alpha, "Alpha"), (beta, "Beta")] {
        db.set_photo_context(&PhotoContext {
            photo_id,
            session_id: None,
            world_id: None,
            world_name: Some(world_name.to_string()),
            instance_id: None,
            link_method: "manual".to_string(),
            confidence: 1.0,
            author: None,
            linked_at: now_string(),
        })
        .unwrap();
    }

    let entries = db.photo_index(PhotoSort::World).unwrap();
    assert_eq!(
        entries.iter().map(|entry| entry.id).collect::<Vec<_>>(),
        vec![alpha, beta]
    );
}

#[test]
fn photo_index_falls_back_to_utc_month_for_bad_local_time() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    // 2025-05-01 00:00:00 UTC
    db.upsert_photo(&photo_at(root, "/photos/bad-local.png", 1_746_057_600, ""))
        .unwrap();
    let entries = db.photo_index(PhotoSort::Newest).unwrap();
    assert_eq!(entries[0].captured_month, 202505);
}

#[test]
fn photo_index_includes_session_id_and_local_capture_day() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let world = db
        .upsert_world("wrld_test", Some("Test World"), Some("log"))
        .unwrap();
    let session = db.create_session("log.txt", 100, world, None).unwrap();
    let linked = db
        .upsert_photo(&photo_at(
            root,
            "/photos/linked.png",
            100,
            "2025-04-30 23:41:00.000",
        ))
        .unwrap();
    db.set_photo_context(&PhotoContext {
        photo_id: linked,
        session_id: Some(session),
        world_id: Some(world),
        world_name: Some("Test World".into()),
        instance_id: None,
        link_method: "log".into(),
        confidence: 0.9,
        author: None,
        linked_at: now_string(),
    })
    .unwrap();
    let unlinked = db
        .upsert_photo(&photo_at(
            root,
            "/photos/unlinked.png",
            99,
            "2025-04-30 22:00:00.000",
        ))
        .unwrap();

    let entries = db.photo_index(PhotoSort::Newest).unwrap();
    let linked_entry = entries.iter().find(|entry| entry.id == linked).unwrap();
    assert_eq!(linked_entry.session_id, Some(session));
    assert_eq!(linked_entry.captured_day, 20250430);
    let unlinked_entry = entries.iter().find(|entry| entry.id == unlinked).unwrap();
    assert_eq!(unlinked_entry.session_id, None);
    assert_eq!(unlinked_entry.captured_day, 20250430);
}

#[test]
fn photo_index_day_falls_back_to_utc_when_local_time_is_invalid() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let id = db
        .upsert_photo(&photo_at(root, "/photos/a.png", 0, "not-a-date"))
        .unwrap();
    // `photo_at` sets captured_at_utc to 0 => 1970-01-01 UTC.
    let entry = db
        .photo_index(PhotoSort::Newest)
        .unwrap()
        .into_iter()
        .find(|entry| entry.id == id)
        .unwrap();
    assert_eq!(entry.captured_day, 19700101);
}

#[test]
fn photo_index_text_returns_more_than_300_matches() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    for index in 0..305 {
        db.upsert_photo(&photo_at(
            root,
            &format!("/photos/{index}.png"),
            1_700_000_000 + index,
            "2025-04-30 09:00:00",
        ))
        .unwrap();
    }
    let entries = db.photo_index_text(&PhotoTextFilter::default()).unwrap();
    assert_eq!(entries.len(), 305);
}

fn set_context(db: &Db, photo_id: i64, world_name: Option<&str>) {
    db.set_photo_context(&PhotoContext {
        photo_id,
        session_id: None,
        world_id: None,
        world_name: world_name.map(str::to_string),
        instance_id: None,
        link_method: "manual".to_string(),
        confidence: 1.0,
        author: None,
        linked_at: now_string(),
    })
    .unwrap();
}

#[test]
fn world_suggestions_match_partial_names_with_visible_counts() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let a = db
        .upsert_photo(&photo_at(root, "/photos/a.png", 1, "2025-04-30 09:00:00"))
        .unwrap();
    let b = db
        .upsert_photo(&photo_at(root, "/photos/b.png", 2, "2025-04-30 09:00:00"))
        .unwrap();
    let c = db
        .upsert_photo(&photo_at(root, "/photos/c.png", 3, "2025-04-30 09:00:00"))
        .unwrap();
    let missing = db
        .upsert_photo(&photo_at(root, "/photos/m.png", 4, "2025-04-30 09:00:00"))
        .unwrap();
    for (photo_id, name) in [
        (a, "Sunset Pier"),
        (b, "Sunset Pier"),
        (c, "Sunset Bar"),
        (missing, "Sunset Missing"),
    ] {
        set_context(&db, photo_id, Some(name));
    }
    db.mark_missing(missing).unwrap();

    let rows = db.world_name_suggestions("sun", 10).unwrap();
    assert_eq!(
        rows,
        vec![
            SearchSuggestion {
                name: "Sunset Pier".into(),
                photo_count: 2,
            },
            SearchSuggestion {
                name: "Sunset Bar".into(),
                photo_count: 1,
            },
        ]
    );
    assert!(db.world_name_suggestions("  ", 10).unwrap().is_empty());
}

#[test]
fn photo_players_many_groups_by_photo_and_keeps_name_order() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let a = db
        .upsert_photo(&photo_at(root, "/photos/a.png", 1, "2025-04-30 09:00:00"))
        .unwrap();
    let b = db
        .upsert_photo(&photo_at(root, "/photos/b.png", 2, "2025-04-30 09:00:00"))
        .unwrap();
    db.set_photo_players(
        a,
        &[
            PhotoPlayer {
                user_id: "usr_2".into(),
                display_name: "Beta".into(),
            },
            PhotoPlayer {
                user_id: "usr_1".into(),
                display_name: "alpha".into(),
            },
        ],
    )
    .unwrap();
    db.set_photo_players(
        b,
        &[PhotoPlayer {
            user_id: "usr_3".into(),
            display_name: "Gamma".into(),
        }],
    )
    .unwrap();

    let grouped = db.photo_players_many(&[a, b, 999]).unwrap();
    assert_eq!(
        grouped[&a]
            .iter()
            .map(|player| player.display_name.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "Beta"]
    );
    assert_eq!(grouped[&b][0].display_name, "Gamma");
    assert!(!grouped.contains_key(&999));
}

#[test]
fn player_suggestions_group_names_and_count_distinct_photos() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let a = db
        .upsert_photo(&photo_at(root, "/photos/a.png", 1, "2025-04-30 09:00:00"))
        .unwrap();
    let b = db
        .upsert_photo(&photo_at(root, "/photos/b.png", 2, "2025-04-30 09:00:00"))
        .unwrap();
    let c = db
        .upsert_photo(&photo_at(root, "/photos/c.png", 3, "2025-04-30 09:00:00"))
        .unwrap();
    db.set_photo_players(
        a,
        &[PhotoPlayer {
            user_id: "usr_1".into(),
            display_name: "Mio".into(),
        }],
    )
    .unwrap();
    db.set_photo_players(
        b,
        &[PhotoPlayer {
            user_id: "usr_2".into(),
            display_name: "Mio".into(),
        }],
    )
    .unwrap();
    db.set_photo_players(
        c,
        &[PhotoPlayer {
            user_id: "usr_3".into(),
            display_name: "Mika".into(),
        }],
    )
    .unwrap();

    let rows = db.player_name_suggestions("mi", 10).unwrap();
    assert_eq!(
        rows,
        vec![
            SearchSuggestion {
                name: "Mio".into(),
                photo_count: 2,
            },
            SearchSuggestion {
                name: "Mika".into(),
                photo_count: 1,
            },
        ]
    );
}

#[test]
fn suggestions_escape_like_metacharacters_and_honor_the_limit() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    for (index, name) in ["100%_World", "100XWorld", "Percent"].iter().enumerate() {
        let photo = db
            .upsert_photo(&photo_at(
                root,
                &format!("/photos/{index}.png"),
                index as i64,
                "2025-04-30 09:00:00",
            ))
            .unwrap();
        set_context(&db, photo, Some(name));
    }

    let rows = db.world_name_suggestions("100%", 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "100%_World");
    assert_eq!(db.world_name_suggestions("o", 1).unwrap().len(), 1);
}

fn seed_photo_at(db: &Db, path: &str) -> i64 {
    let root = db.add_root("/photos", "reference").unwrap();
    db.upsert_photo(&sample_photo(root, path, 1_700_000_000))
        .unwrap()
}

#[test]
fn tag_suggestions_merge_manual_and_applied_ai_tags() {
    let db = Db::open_in_memory().unwrap();
    install_test_catalog(&db);
    let a = seed_photo(&db);
    let b = seed_photo_at(&db, "/photos/b.png");
    let c = seed_photo_at(&db, "/photos/c.png");
    db.set_photo_tags(a, &["夜景".into()]).unwrap();
    db.set_photo_tags(b, &["夜景".into()]).unwrap();
    seed_ai_tag(&db, c, AI_PLUGIN, "sea", 0.97, true);
    seed_ai_tag(&db, c, AI_PLUGIN, "park", 0.97, false);

    assert_eq!(
        db.tag_name_suggestions("夜", 10, false).unwrap(),
        vec![SearchSuggestion {
            name: "夜景".into(),
            photo_count: 2,
        }]
    );
    assert_eq!(
        db.tag_name_suggestions("海", 10, false).unwrap(),
        vec![SearchSuggestion {
            name: "海".into(),
            photo_count: 1,
        }]
    );
    assert_eq!(
        db.tag_name_suggestions("Sea", 10, true).unwrap(),
        vec![SearchSuggestion {
            name: "Sea".into(),
            photo_count: 1,
        }]
    );
    assert!(db.tag_name_suggestions("公", 10, false).unwrap().is_empty());
}

#[test]
fn tag_suggestions_merge_same_name_and_skip_unapplied() {
    let db = Db::open_in_memory().unwrap();
    install_test_catalog(&db);
    let a = seed_photo(&db);
    let b = seed_photo_at(&db, "/photos/b.png");
    db.set_photo_tags(a, &["海".into()]).unwrap();
    seed_ai_tag(&db, a, AI_PLUGIN, "sea", 0.97, true);
    seed_ai_tag(&db, b, AI_PLUGIN, "sea", 0.97, true);

    let rows = db.tag_name_suggestions("海", 10, false).unwrap();
    assert_eq!(
        rows,
        vec![SearchSuggestion {
            name: "海".into(),
            photo_count: 2,
        }]
    );
}

#[test]
fn tag_text_filter_matches_applied_ai_tag_names_and_keys() {
    let db = Db::open_in_memory().unwrap();
    install_test_catalog(&db);
    let photo = seed_photo(&db);
    seed_ai_tag(&db, photo, AI_PLUGIN, "sea", 0.97, true);

    for query in ["海", "Sea", "sea"] {
        let filter = PhotoTextFilter {
            tag: Some(query.into()),
            ..Default::default()
        };
        assert_eq!(db.photo_index_text(&filter).unwrap().len(), 1, "{query}");
        assert_eq!(
            db.search_photos_text(&filter, 10, 0).unwrap().len(),
            1,
            "{query}"
        );
    }

    let unapplied = seed_photo_at(&db, "/photos/unapplied.png");
    seed_ai_tag(&db, unapplied, AI_PLUGIN, "park", 0.97, false);
    let filter = PhotoTextFilter {
        tag: Some("公園".into()),
        ..Default::default()
    };
    assert!(db.photo_index_text(&filter).unwrap().is_empty());
}

#[test]
fn ai_tag_text_search_escapes_like_metacharacters() {
    let db = Db::open_in_memory().unwrap();
    let catalog = Catalog {
        version: "2026-11-03.1".into(),
        outdoor: vec![tag("percent", "100%_tag", Some("100%_tag"), 0.5)],
        indoor: Vec::new(),
        atmosphere: Vec::new(),
    };
    db.sync_ai_catalog(AI_PLUGIN, &catalog).unwrap();
    let photo = seed_photo(&db);
    seed_ai_tag(&db, photo, AI_PLUGIN, "percent", 0.97, true);

    let filter = PhotoTextFilter {
        tag: Some("100%".into()),
        ..Default::default()
    };
    assert_eq!(db.photo_index_text(&filter).unwrap().len(), 1);
}

#[test]
fn photo_index_text_treats_like_metacharacters_literally() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let percent = db
        .upsert_photo(&sample_photo(root, "/photos/percent.png", 100))
        .unwrap();
    let underscore = db
        .upsert_photo(&sample_photo(root, "/photos/underscore.png", 200))
        .unwrap();
    let plain = db
        .upsert_photo(&sample_photo(root, "/photos/plain.png", 300))
        .unwrap();
    db.set_photo_tags(percent, &["%".to_string()]).unwrap();
    db.set_photo_tags(underscore, &["_".to_string()]).unwrap();
    db.set_photo_tags(plain, &["plain".to_string()]).unwrap();

    let percent_matches = db
        .photo_index_text(&PhotoTextFilter {
            tag: Some("%".to_string()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        percent_matches
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec![percent]
    );

    let underscore_matches = db
        .photo_index_text(&PhotoTextFilter {
            tag: Some("_".to_string()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        underscore_matches
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec![underscore]
    );
}

#[test]
fn photo_index_text_limited_stops_at_the_requested_row_count() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    for index in 0..3 {
        db.upsert_photo(&sample_photo(
            root,
            &format!("/photos/{index}.png"),
            100 + index,
        ))
        .unwrap();
    }
    let entries = db
        .photo_index_text_limited(&PhotoTextFilter::default(), 2)
        .unwrap();
    assert_eq!(entries.len(), 2);
}

#[test]
fn photo_index_text_applies_favorite_filter() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let favorite = db
        .upsert_photo(&photo_at(
            root,
            "/photos/favorite.png",
            1_700_000_100,
            "2025-04-30 09:00:00",
        ))
        .unwrap();
    db.upsert_photo(&photo_at(
        root,
        "/photos/plain.png",
        1_700_000_000,
        "2025-04-30 08:00:00",
    ))
    .unwrap();
    db.set_favorite(favorite, true).unwrap();

    let entries = db
        .photo_index_text(&PhotoTextFilter {
            favorite: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        entries.iter().map(|entry| entry.id).collect::<Vec<_>>(),
        vec![favorite]
    );
}

#[test]
fn set_note_round_trips_and_clears_with_none() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let id = db
        .upsert_photo(&photo_at(
            root,
            "/photos/note.png",
            1_700_000_000,
            "2025-04-30 09:00:00",
        ))
        .unwrap();

    assert_eq!(db.photo_by_id(id).unwrap().unwrap().note, None);
    db.set_note(id, Some("first note")).unwrap();
    assert_eq!(
        db.photo_by_id(id).unwrap().unwrap().note.as_deref(),
        Some("first note")
    );
    db.set_note(id, None).unwrap();
    assert_eq!(db.photo_by_id(id).unwrap().unwrap().note, None);
}

#[test]
fn removing_a_tag_drops_the_photo_from_tag_search() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let id = db
        .upsert_photo(&photo_at(
            root,
            "/photos/tagged.png",
            1_700_000_000,
            "2025-04-30 09:00:00",
        ))
        .unwrap();
    db.set_photo_tags(id, &["keep".to_string(), "remove".to_string()])
        .unwrap();

    let filter = PhotoTextFilter {
        tag: Some("remove".to_string()),
        ..Default::default()
    };
    assert_eq!(db.photo_index_text(&filter).unwrap().len(), 1);

    db.set_photo_tags(id, &["keep".to_string()]).unwrap();
    assert!(db.photo_index_text(&filter).unwrap().is_empty());
    assert_eq!(db.photo_tags(id).unwrap(), vec!["keep".to_string()]);
}

#[test]
fn add_tags_to_photos_updates_every_selected_photo_once() {
    let db = Db::open_in_memory().unwrap();
    let photos = seed_photos(&db, 3);

    db.add_tags_to_photos(
        &[photos[0], photos[0], photos[1], photos[2]],
        &[
            " keep ".to_string(),
            String::new(),
            "keep".to_string(),
            "new".to_string(),
        ],
    )
    .unwrap();

    for id in &photos {
        assert_eq!(
            db.photo_tags(*id).unwrap(),
            vec!["keep".to_string(), "new".to_string()]
        );
    }

    db.add_tags_to_photos(&photos, &["keep".to_string(), "other".to_string()])
        .unwrap();
    for id in &photos {
        assert_eq!(
            db.photo_tags(*id).unwrap(),
            vec!["keep".to_string(), "new".to_string(), "other".to_string()]
        );
    }
}

#[test]
fn remove_tags_from_photos_keeps_other_tags_and_ignores_unknown_tags() {
    let db = Db::open_in_memory().unwrap();
    let photos = seed_photos(&db, 3);
    db.set_photo_tags(photos[0], &["keep".to_string(), "remove".to_string()])
        .unwrap();
    db.set_photo_tags(
        photos[1],
        &[
            "keep".to_string(),
            "remove".to_string(),
            "other".to_string(),
        ],
    )
    .unwrap();
    db.set_photo_tags(photos[2], &["remove".to_string()])
        .unwrap();

    db.remove_tags_from_photos(&photos, &[" remove ".to_string(), "missing".to_string()])
        .unwrap();

    assert_eq!(db.photo_tags(photos[0]).unwrap(), vec!["keep".to_string()]);
    assert_eq!(
        db.photo_tags(photos[1]).unwrap(),
        vec!["keep".to_string(), "other".to_string()]
    );
    assert!(db.photo_tags(photos[2]).unwrap().is_empty());
}

#[test]
fn update_tags_for_photos_adds_and_removes_in_one_call() {
    let db = Db::open_in_memory().unwrap();
    let photos = seed_photos(&db, 2);
    for id in &photos {
        db.set_photo_tags(*id, &["keep".to_string(), "remove".to_string()])
            .unwrap();
    }

    db.update_tags_for_photos(
        &photos,
        &[" add ".to_string(), "add".to_string()],
        &["remove".to_string()],
    )
    .unwrap();

    for id in &photos {
        assert_eq!(
            db.photo_tags(*id).unwrap(),
            vec!["add".to_string(), "keep".to_string()]
        );
    }
}

#[test]
fn update_tags_for_photos_rolls_back_when_a_photo_is_missing() {
    let db = Db::open_in_memory().unwrap();
    let photos = seed_photos(&db, 2);
    db.set_photo_tags(photos[0], &["keep".to_string()]).unwrap();
    let missing = photos[1] + 10_000;

    assert!(
        db.update_tags_for_photos(
            &[photos[0], missing],
            &["add".to_string()],
            &["keep".to_string()],
        )
        .is_err()
    );
    assert_eq!(db.photo_tags(photos[0]).unwrap(), vec!["keep".to_string()]);
}

#[test]
fn common_manual_tags_are_the_intersection_in_name_order() {
    let db = Db::open_in_memory().unwrap();
    let photos = seed_photos(&db, 3);
    db.set_photo_tags(
        photos[0],
        &["shared".to_string(), "a".to_string(), "b".to_string()],
    )
    .unwrap();
    db.set_photo_tags(
        photos[1],
        &["shared".to_string(), "a".to_string(), "c".to_string()],
    )
    .unwrap();
    db.set_photo_tags(
        photos[2],
        &["shared".to_string(), "a".to_string(), "d".to_string()],
    )
    .unwrap();

    assert_eq!(
        db.common_manual_tags(&photos).unwrap(),
        vec!["a".to_string(), "shared".to_string()]
    );
    assert!(
        db.common_manual_tags(&[]).unwrap().is_empty(),
        "an empty selection has no common tags"
    );
    assert_eq!(
        db.common_manual_tags(&photos[..1]).unwrap(),
        vec!["a".to_string(), "b".to_string(), "shared".to_string()]
    );
}

#[test]
fn bulk_tag_writes_leave_no_partial_update_when_a_photo_is_missing() {
    let db = Db::open_in_memory().unwrap();
    let photos = seed_photos(&db, 2);
    let missing = photos[1] + 10_000;

    assert!(
        db.add_tags_to_photos(&[photos[0], missing], &["new".to_string()])
            .is_err()
    );
    assert!(db.photo_tags(photos[0]).unwrap().is_empty());
    assert!(db.photo_tags(photos[1]).unwrap().is_empty());

    db.set_photo_tags(photos[0], &["keep".to_string()]).unwrap();
    assert!(
        db.remove_tags_from_photos(&[photos[0], missing], &["keep".to_string()])
            .is_err()
    );
    assert_eq!(db.photo_tags(photos[0]).unwrap(), vec!["keep".to_string()]);
    assert!(
        db.common_manual_tags(&[photos[0], missing]).is_err(),
        "common tags must not silently ignore a missing photo"
    );
}

#[test]
fn bulk_tag_writes_do_not_touch_ai_tags() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    install_test_catalog(&db);
    db.save_photo_ai(
        photo,
        AI_PLUGIN,
        &test_ai_result(&[("sea", 0.97), ("park", 0.42)]),
    )
    .unwrap();
    let before = db.photo_ai_tags(photo, false).unwrap();

    db.add_tags_to_photos(&[photo], &["manual".to_string()])
        .unwrap();
    db.remove_tags_from_photos(&[photo], &["manual".to_string()])
        .unwrap();

    assert_eq!(db.photo_ai_tags(photo, false).unwrap(), before);
}

#[test]
fn bulk_tag_writes_and_common_tags_accept_empty_inputs() {
    let db = Db::open_in_memory().unwrap();
    db.add_tags_to_photos(&[], &["new".to_string()]).unwrap();
    db.add_tags_to_photos(&[seed_photo(&db)], &[]).unwrap();
    db.remove_tags_from_photos(&[], &["remove".to_string()])
        .unwrap();
    assert!(db.common_manual_tags(&[]).unwrap().is_empty());
}

#[test]
fn photos_by_ids_preserves_order_and_skips_missing_ids() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let mut ids = Vec::new();
    for index in 0..501 {
        ids.push(
            db.upsert_photo(&photo_at(
                root,
                &format!("/photos/{index}.png"),
                1_700_000_000 + index,
                "2025-04-30 09:00:00",
            ))
            .unwrap(),
        );
    }

    let photos = db.photos_by_ids(&ids).unwrap();
    assert_eq!(photos.len(), 501);
    assert_eq!(photos.first().map(|photo| photo.id), Some(ids[0]));
    assert_eq!(photos.last().map(|photo| photo.id), Some(ids[500]));

    // The reversed request crosses the 500-id chunk boundary; every row
    // must come back in exactly the requested order.
    let reversed: Vec<i64> = ids.iter().rev().copied().collect();
    let photos = db.photos_by_ids(&reversed).unwrap();
    assert_eq!(
        photos.iter().map(|photo| photo.id).collect::<Vec<_>>(),
        reversed
    );

    let missing = db.photos_by_ids(&[ids[500], ids[0], -1]).unwrap();
    assert_eq!(
        missing.iter().map(|photo| photo.id).collect::<Vec<_>>(),
        vec![ids[500], ids[0]]
    );
}

#[test]
fn photo_scan_states_reads_all_chunks_with_metadata() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let mut paths = Vec::new();
    let mut first_id = 0;
    let mut missing_id = 0;
    for index in 0..501i64 {
        let path = format!("/photos/{index:04}.png");
        let id = db
            .upsert_photo(&NewPhoto {
                root_id: root,
                rel_path: format!("{index:04}.png"),
                abs_path: path.clone(),
                file_name: format!("{index:04}.png"),
                captured_at_utc: 1_700_000_000 + index,
                captured_at_local: "2026-10-21 21:14:03".to_string(),
                tz_offset_sec: 0,
                width: 4,
                height: 3,
                file_size: 100 + index as u64,
                mtime: 200 + index,
                blake3: (index == 0).then(|| "hash".to_string()),
            })
            .unwrap();
        if index == 0 {
            first_id = id;
        }
        if index == 500 {
            missing_id = id;
        }
        paths.push(path);
    }
    db.mark_missing(missing_id).unwrap();

    let states = db.photo_scan_states(&paths).unwrap();
    assert_eq!(states.len(), 501);
    let first = &states["/photos/0000.png"];
    assert_eq!(first.id, first_id);
    assert_eq!(first.file_size, 100);
    assert_eq!(first.mtime, 200);
    assert_eq!(first.blake3.as_deref(), Some("hash"));
    assert_eq!(first.status, PhotoStatus::Present);
    assert_eq!(first.display_derivative_id, None);
    assert_eq!(states["/photos/0500.png"].status, PhotoStatus::Missing);

    let derivative = db
        .add_derivative(
            first_id,
            "/photos/0000_webp.webp",
            "webp",
            4,
            3,
            10,
            Some("p"),
            CompressionOrigin::Manual,
            "replace",
        )
        .unwrap();
    db.set_replaced(first_id, derivative).unwrap();
    let states = db
        .photo_scan_states(&["/photos/0000.png".to_string()])
        .unwrap();
    let replaced = &states["/photos/0000.png"];
    assert_eq!(replaced.id, first_id);
    assert_eq!(replaced.display_derivative_id, Some(derivative));
    assert_eq!(replaced.status, PhotoStatus::Replaced);
}

/// Seeds 100k synthetic rows in one transaction and measures the shipped
/// index and page queries. Ignored by default because it is a timing
/// benchmark, not a correctness gate:
///
/// `cargo test -p vrc-photon-core photo_index_scales_to_100k -- --ignored --nocapture`
#[test]
#[ignore = "100k timing benchmark"]
fn photo_index_scales_to_100k() {
    use std::time::{Duration, Instant};

    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let base = 1_600_000_000i64;
    let seed_start = Instant::now();
    {
        let tx = db.connection().unchecked_transaction().unwrap();
        {
            let mut stmt = tx
                .prepare_cached(
                    "INSERT INTO photos(root_id, rel_path, abs_path, file_name, \
                         captured_at_utc, captured_at_local, tz_offset_sec, width, height, \
                         file_size, mtime, status, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 3840, 2160, 12400000, ?5, \
                         'present', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
                )
                .unwrap();
            for index in 0..100_000i64 {
                let captured_at = base + index;
                let local = chrono::DateTime::<chrono::Utc>::from_timestamp(captured_at, 0)
                    .unwrap()
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string();
                let rel_path = format!("2026/{index:06}.png");
                let abs_path = format!("/photos/{rel_path}");
                stmt.execute(params![
                    root,
                    rel_path,
                    abs_path,
                    format!("{index:06}.png"),
                    captured_at,
                    local
                ])
                .unwrap();
            }
        }
        // The shipped query LEFT JOINs `photo_context`; seed every other
        // row so the benchmark measures the production join instead of an
        // all-NULL shortcut.
        tx.execute(
            "INSERT INTO photo_context(photo_id, world_name, link_method, confidence, linked_at) \
                 SELECT id, 'Bench World ' || (id % 100), 'log', 1.0, \
                 '2026-01-01T00:00:00+00:00' FROM photos WHERE id % 2 = 0",
            [],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO photo_ai(photo_id, plugin_id, variant, sidecar_version, \
                    model_version, model_revision, catalog_version, source, \
                    presence_threshold, character_present, presence_probability, \
                    people_min, people_max, people_median, place_branch, \
                    place_probabilities_json, tagged_at) \
                 SELECT id, ?1, 'bench', '1.0.0', 'd1-2B', 'rev', '2026-11-03.1', \
                    'original', 0.9, 1, 0.99, 2, 4, 3, 'outdoor', '{}', \
                    '2026-11-04T00:00:00+00:00' \
                 FROM photos WHERE id % 4 = 0",
            params![AI_PLUGIN_ID],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO photo_ai_tags(photo_id, plugin_id, tag_key, probability, \
                    applied, model_version, catalog_version, tagged_at) \
                 SELECT photo_id, ?1, 'sea', 0.9, 1, 'd1-2B', '2026-11-03.1', \
                    '2026-11-04T00:00:00+00:00' \
                 FROM photo_ai WHERE plugin_id = ?1",
            params![AI_PLUGIN_ID],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    eprintln!("seed 100k: {:?}", seed_start.elapsed());
    let context_count: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM photo_context", [], |row| row.get(0))
        .unwrap();
    assert_eq!(context_count, 50_000);

    let start = Instant::now();
    let entries = db.photo_index(PhotoSort::Newest).unwrap();
    let index_elapsed = start.elapsed();
    assert_eq!(entries.len(), 100_000);
    eprintln!("photo_index 100k: {index_elapsed:?}");
    assert!(
        index_elapsed < Duration::from_secs(2),
        "photo_index took {index_elapsed:?}"
    );

    let ids: Vec<i64> = entries[50_000..50_256]
        .iter()
        .map(|entry| entry.id)
        .collect();
    let start = Instant::now();
    let photos = db.photos_by_ids(&ids).unwrap();
    let page_elapsed = start.elapsed();
    eprintln!("photos_by_ids 256: {page_elapsed:?}");
    assert_eq!(photos.len(), 256);
    assert!(
        page_elapsed < Duration::from_millis(200),
        "photos_by_ids took {page_elapsed:?}"
    );

    // The search screen builds the same full snapshot with a text filter;
    // an empty filter is the worst case (all 100k rows match).
    let start = Instant::now();
    let matches = db.photo_index_text(&PhotoTextFilter::default()).unwrap();
    eprintln!("photo_index_text 100k: {:?}", start.elapsed());
    assert_eq!(matches.len(), 100_000);

    let ai_filter = PhotoTextFilter {
        ai: AiPhotoFilter {
            tags: vec!["sea".into()],
            people_count: Some(3),
        },
        ..Default::default()
    };
    let start = Instant::now();
    let ai_matches = db.photo_index_text(&ai_filter).unwrap();
    let ai_elapsed = start.elapsed();
    eprintln!("photo_index_text AI 100k: {ai_elapsed:?}");
    assert_eq!(ai_matches.len(), 25_000);
    assert!(
        ai_elapsed < Duration::from_secs(2),
        "photo_index_text AI took {ai_elapsed:?}"
    );
}

#[test]
fn connections_use_a_large_page_cache_and_memory_temp_store() {
    let db = Db::open_in_memory().unwrap();
    let cache_size: i64 = db
        .conn
        .pragma_query_value(None, "cache_size", |row| row.get(0))
        .unwrap();
    assert_eq!(cache_size, -65_536);
    let temp_store: i64 = db
        .conn
        .pragma_query_value(None, "temp_store", |row| row.get(0))
        .unwrap();
    assert_eq!(temp_store, 2);
}

#[test]
fn ui_connections_use_a_smaller_cache_and_mmap_window() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let db = Db::open_with_profile(&path, DbProfile::Ui).unwrap();
    let cache_size: i64 = db
        .conn
        .pragma_query_value(None, "cache_size", |row| row.get(0))
        .unwrap();
    assert_eq!(cache_size, -16_384);
    let mmap_size: i64 = db
        .conn
        .pragma_query_value(None, "mmap_size", |row| row.get(0))
        .unwrap();
    assert_eq!(mmap_size, 67_108_864);
}

#[test]
fn read_only_connections_accept_a_profile() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    drop(Db::open(&path).unwrap());

    let db = Db::open_read_only_with_profile(&path, DbProfile::Ui).unwrap();
    let cache_size: i64 = db
        .conn
        .pragma_query_value(None, "cache_size", |row| row.get(0))
        .unwrap();
    assert_eq!(cache_size, -16_384);
}

#[test]
fn database_path_is_exposed_for_read_only_workers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let db = Db::open(&path).unwrap();
    assert_eq!(db.path(), Some(path.as_path()));
    assert_eq!(Db::open_in_memory().unwrap().path(), None);
}

#[test]
fn migrates_and_upserts_photo() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let id = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    assert!(id > 0);
    let photo = db.photo_by_id(id).unwrap().unwrap();
    assert_eq!(photo.width, 3840);
    assert_eq!(photo.status, PhotoStatus::Present);

    let id2 = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    assert_eq!(id, id2);
    assert_eq!(db.count_photos().unwrap(), 1);
}

#[test]
fn links_context_and_players() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo_id = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let world = db
        .upsert_world("wrld_test", Some("Test World"), Some("log"))
        .unwrap();
    let session = db
        .create_session("output_log.txt", 90, world, Some("123~region(jp)"))
        .unwrap();
    db.upsert_session_player(session, "usr_a", "Alice", 95)
        .unwrap();

    let ctx = PhotoContext {
        photo_id,
        session_id: Some(session),
        world_id: Some(world),
        world_name: Some("Test World".to_string()),
        instance_id: Some("123~region(jp)".to_string()),
        link_method: "log".to_string(),
        confidence: 1.0,
        author: None,
        linked_at: now_string(),
    };
    db.set_photo_context(&ctx).unwrap();
    db.set_photo_players(
        photo_id,
        &[PhotoPlayer {
            user_id: "usr_a".to_string(),
            display_name: "Alice".to_string(),
        }],
    )
    .unwrap();

    let stored = db.photo_context(photo_id).unwrap().unwrap();
    assert_eq!(stored.world_name.as_deref(), Some("Test World"));
    assert_eq!(db.photo_players(photo_id).unwrap()[0].display_name, "Alice");

    let filter = PhotoFilter {
        world_ids: vec![world],
        ..Default::default()
    };
    assert_eq!(db.search_photos(&filter).unwrap().len(), 1);
    assert_eq!(db.count_search(&filter).unwrap(), 1);
}

#[test]
fn open_sessions_returns_only_unfinished_sessions() {
    let db = Db::open_in_memory().unwrap();
    let world = db.upsert_world("wrld_test", None, None).unwrap();
    let open = db.create_session("open.txt", 10, world, None).unwrap();
    let closed = db.create_session("closed.txt", 20, world, None).unwrap();
    db.close_session(closed, 30).unwrap();

    let sessions = db.open_sessions().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, open);
}

#[test]
fn session_lifecycle_bumps_session_revision_without_moving_library_revision() {
    let db = Db::open_in_memory().unwrap();
    let world = db
        .upsert_world("wrld_test", Some("Test World"), Some("log"))
        .unwrap();
    let library_before = db.library_revision().unwrap();
    let revision_before = db.session_revision().unwrap();

    let session = db.create_session("log.txt", 100, world, None).unwrap();
    assert!(db.session_revision().unwrap() > revision_before);

    let revision_before = db.session_revision().unwrap();
    db.close_session(session, 200).unwrap();
    assert!(db.session_revision().unwrap() > revision_before);

    let revision_before = db.session_revision().unwrap();
    db.close_session(session, 200).unwrap();
    assert_eq!(db.session_revision().unwrap(), revision_before);

    let revision_before = db.session_revision().unwrap();
    db.upsert_session_player(session, "usr_a", "Aoi", 110)
        .unwrap();
    assert!(db.session_revision().unwrap() > revision_before);

    let revision_before = db.session_revision().unwrap();
    db.close_session_player(session, "usr_a", 120).unwrap();
    assert!(db.session_revision().unwrap() > revision_before);

    assert_eq!(db.library_revision().unwrap(), library_before);
}

#[test]
fn session_summaries_return_world_count_players_and_cover() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let world = db
        .upsert_world("wrld_test", Some("Test World"), Some("log"))
        .unwrap();
    let session = db
        .create_session("log.txt", 100, world, Some("123~region(jp)"))
        .unwrap();
    db.close_session(session, 200).unwrap();
    db.upsert_session_player(session, "usr_b", "Ren", 120)
        .unwrap();
    db.upsert_session_player(session, "usr_a", "Aoi", 110)
        .unwrap();

    let older = db
        .upsert_photo(&photo_at(
            root,
            "/photos/older.png",
            130,
            "2025-04-30 09:00:00",
        ))
        .unwrap();
    let favorite = db
        .upsert_photo(&photo_at(
            root,
            "/photos/fav.png",
            140,
            "2025-04-30 09:01:00",
        ))
        .unwrap();
    let newest = db
        .upsert_photo(&photo_at(
            root,
            "/photos/newest.png",
            150,
            "2025-04-30 09:02:00",
        ))
        .unwrap();
    for id in [older, favorite, newest] {
        db.set_photo_context(&PhotoContext {
            photo_id: id,
            session_id: Some(session),
            world_id: Some(world),
            world_name: Some("Test World".into()),
            instance_id: Some("123~region(jp)".into()),
            link_method: "log".into(),
            confidence: 0.9,
            author: None,
            linked_at: now_string(),
        })
        .unwrap();
    }
    db.set_favorite(favorite, true).unwrap();

    let summaries = db.session_summaries(&[session]).unwrap();
    assert_eq!(summaries.len(), 1);
    let summary = &summaries[0];
    assert_eq!(summary.session.id, session);
    assert_eq!(summary.world_name.as_deref(), Some("Test World"));
    assert_eq!(summary.world_external_id.as_deref(), Some("wrld_test"));
    assert_eq!(summary.photo_count, 3);
    assert_eq!(summary.players, vec!["Aoi".to_string(), "Ren".to_string()]);
    assert_eq!(
        summary.cover_photo.as_ref().map(|photo| photo.id),
        Some(favorite)
    );
}

#[test]
fn session_summaries_skip_unknown_ids_and_count_only_visible_photos() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let world = db.upsert_world("wrld_test", None, None).unwrap();
    let session = db.create_session("log.txt", 100, world, None).unwrap();
    let missing = db
        .upsert_photo(&photo_at(
            root,
            "/photos/missing.png",
            120,
            "2025-04-30 09:00:00",
        ))
        .unwrap();
    db.set_photo_context(&PhotoContext {
        photo_id: missing,
        session_id: Some(session),
        world_id: Some(world),
        world_name: None,
        instance_id: None,
        link_method: "log".into(),
        confidence: 0.9,
        author: None,
        linked_at: now_string(),
    })
    .unwrap();
    db.mark_missing(missing).unwrap();

    let summaries = db.session_summaries(&[session, 9999]).unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].photo_count, 0);
    assert_eq!(summaries[0].world_name.as_deref(), Some("wrld_test"));
    assert!(summaries[0].cover_photo.is_none());
}

#[test]
fn session_summaries_deduplicate_ids_and_accept_many_ids() {
    let db = Db::open_in_memory().unwrap();
    assert!(db.session_summaries(&[]).unwrap().is_empty());
    let world = db.upsert_world("wrld_test", None, None).unwrap();
    let session = db.create_session("log.txt", 100, world, None).unwrap();
    assert_eq!(db.session_summaries(&[session, session]).unwrap().len(), 1);
    let many: Vec<i64> = (session + 1..=session + 300).collect();
    assert!(db.session_summaries(&many).unwrap().is_empty());
}

#[test]
fn sessions_near_uses_inclusive_start_and_end_boundaries() {
    let cases = [
        ("ends at the timestamp", 900, Some(1000), true),
        ("ends at the lower bound", 900, Some(970), true),
        ("ends before the lower bound", 900, Some(969), false),
        ("starts at the timestamp", 1000, None, true),
        ("starts at the upper bound", 1030, None, true),
        ("starts after the upper bound", 1031, None, false),
        ("contains the timestamp", 900, Some(1100), false),
        ("is open before the timestamp", 900, None, false),
    ];

    for (label, started_at, ended_at, expected) in cases {
        let db = Db::open_in_memory().unwrap();
        let world = db.upsert_world("wrld_test", None, None).unwrap();
        let session = db
            .create_session("log.txt", started_at, world, None)
            .unwrap();
        if let Some(ended_at) = ended_at {
            db.close_session(session, ended_at).unwrap();
        }

        let near = db.sessions_near(1000, 30).unwrap().len() == 1;
        assert_eq!(near, expected, "{label}");
    }
}

#[test]
fn sessions_near_deduplicates_a_zero_length_session_at_the_timestamp() {
    let db = Db::open_in_memory().unwrap();
    let world = db.upsert_world("wrld_test", None, None).unwrap();
    let session = db.create_session("log.txt", 1000, world, None).unwrap();
    db.close_session(session, 1000).unwrap();

    let near = db.sessions_near(1000, 30).unwrap();
    assert_eq!(near.len(), 1, "UNION must collapse both range matches");
    assert_eq!(near[0].id, session);
}

#[test]
fn sessions_near_orders_by_start_and_returns_at_most_two() {
    let db = Db::open_in_memory().unwrap();
    let world = db.upsert_world("wrld_test", None, None).unwrap();
    for started_at in [1003, 1001, 1002, 1004] {
        db.create_session("log.txt", started_at, world, None)
            .unwrap();
    }

    let near = db.sessions_near(1000, 30).unwrap();
    assert_eq!(
        near.iter()
            .map(|session| session.started_at)
            .collect::<Vec<_>>(),
        vec![1001, 1002]
    );
}

#[test]
fn log_mtime_reads_the_last_ingested_mtime() {
    let db = Db::open_in_memory().unwrap();
    assert_eq!(db.log_mtime("output_log.txt").unwrap(), None);
    db.set_log_offset("output_log.txt", 42, 1_700_000_123, 42, "1")
        .unwrap();
    assert_eq!(db.log_mtime("output_log.txt").unwrap(), Some(1_700_000_123));
}

#[test]
fn queue_roundtrip() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    db.enqueue_job(photo, "profile", "keep").unwrap();
    let job = db.next_pending_job().unwrap().unwrap();
    assert_eq!(job.status, QueueStatus::Processing);
    db.update_job(
        &job.id,
        QueueStatus::Completed,
        1.0,
        None,
        Some("/photos/compressed/a.webp"),
    )
    .unwrap();
    assert_eq!(db.count_jobs(QueueStatus::Completed).unwrap(), 1);
}

#[test]
fn next_pending_photo_jobs_claims_the_oldest_photos_jobs() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let first = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let second = db
        .upsert_photo(&sample_photo(root, "/photos/b.png", 200))
        .unwrap();
    db.save_profile(&profile("p", "P")).unwrap();
    let first_a = db.enqueue_job(first, "p", "keep").unwrap();
    let first_b = db.enqueue_job(first, "p", "keep").unwrap();
    let waiting = db
        .enqueue_job_with_status(first, "p", "keep", QueueStatus::Waiting)
        .unwrap();
    let second_a = db.enqueue_job(second, "p", "keep").unwrap();

    let batch = db.next_pending_photo_jobs().unwrap();
    assert_eq!(
        batch.iter().map(|job| job.id.as_str()).collect::<Vec<_>>(),
        vec![first_a.as_str(), first_b.as_str()]
    );
    assert!(
        batch
            .iter()
            .all(|job| job.status == QueueStatus::Processing)
    );
    assert!(batch.iter().all(|job| job.started_at.is_some()));

    let next = db.next_pending_photo_jobs().unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].id, second_a);
    assert!(db.next_pending_photo_jobs().unwrap().is_empty());

    let waiting = db
        .list_jobs(10)
        .unwrap()
        .into_iter()
        .find(|job| job.id == waiting)
        .unwrap();
    assert_eq!(waiting.status, QueueStatus::Waiting);
}

#[test]
fn photo_has_forced_deletion_tracks_the_force_flag() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    let request = db
        .request_deletion(&[id], DeletionReason::Manual, false)
        .unwrap()[0];

    assert!(!db.photo_has_forced_deletion(id).unwrap());
    assert!(db.force_deletion(request).unwrap());
    assert!(db.photo_has_forced_deletion(id).unwrap());
    db.block_deletion(request, "file_error", None).unwrap();
    assert!(db.photo_has_forced_deletion(id).unwrap());
}

#[test]
fn fail_job_if_active_leaves_terminal_jobs_alone() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);
    db.save_profile(&profile("p", "P")).unwrap();
    let first = db.enqueue_job(id, "p", "keep").unwrap();
    let second = db.enqueue_job(id, "p", "keep").unwrap();
    let third = db.enqueue_job(id, "p", "keep").unwrap();
    let batch = db.next_pending_photo_jobs().unwrap();
    assert_eq!(batch.len(), 3);
    db.update_job(&first, QueueStatus::Completed, 1.0, None, None)
        .unwrap();
    db.update_job(&second, QueueStatus::Cancelled, 0.0, None, None)
        .unwrap();

    assert_eq!(
        db.fail_job_if_active(&third, "boom").unwrap(),
        Some(QueueStatus::Failed)
    );
    assert_eq!(
        db.fail_job_if_active(&first, "late").unwrap(),
        Some(QueueStatus::Completed)
    );
    assert_eq!(
        db.fail_job_if_active(&second, "late").unwrap(),
        Some(QueueStatus::Cancelled)
    );
    assert_eq!(db.fail_job_if_active("missing", "late").unwrap(), None);

    let jobs = db.list_jobs(10).unwrap();
    let by_id = |id: &str| jobs.iter().find(|job| job.id == id).unwrap();
    assert_eq!(by_id(&first).status, QueueStatus::Completed);
    assert!(by_id(&first).error.is_none());
    assert_eq!(by_id(&second).status, QueueStatus::Cancelled);
    assert_eq!(by_id(&third).status, QueueStatus::Failed);
    assert_eq!(by_id(&third).error.as_deref(), Some("boom"));
}

#[test]
fn manual_job_keeps_its_output_directory() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    db.enqueue_job_with_output_dir(photo, "profile", "keep", Some("/exports"))
        .unwrap();
    let job = db.next_pending_job().unwrap().unwrap();
    assert_eq!(job.output_dir.as_deref(), Some("/exports"));
    assert_eq!(job.policy, "keep");
}

#[test]
fn automatic_jobs_leave_the_output_directory_unset() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    db.enqueue_job(photo, "profile", "keep").unwrap();
    let job = db.next_pending_job().unwrap().unwrap();
    assert!(job.output_dir.is_none());
}

#[test]
fn auto_job_enqueue_is_once_per_photo() {
    let db = Db::open_in_memory().unwrap();
    let id = seed_photo(&db);

    assert!(
        db.enqueue_auto_job_with_status(id, "keep", QueueStatus::Pending)
            .unwrap()
    );
    assert!(
        !db.enqueue_auto_job_with_status(id, "keep", QueueStatus::Waiting)
            .unwrap()
    );
    assert_eq!(db.list_jobs(10).unwrap().len(), 1);
}

#[test]
fn waiting_jobs_are_not_dequeued_until_released() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let job = db
        .enqueue_job_with_status(photo, "p", "replace", QueueStatus::Waiting)
        .unwrap();

    assert_eq!(db.count_jobs(QueueStatus::Waiting).unwrap(), 1);
    assert_eq!(db.count_active_jobs().unwrap(), 1);
    assert!(db.next_pending_job().unwrap().is_none());

    assert_eq!(db.release_waiting_jobs("keep").unwrap(), 1);
    assert_eq!(db.count_jobs(QueueStatus::Waiting).unwrap(), 0);
    assert_eq!(db.count_jobs(QueueStatus::Pending).unwrap(), 1);
    let released = db.next_pending_job().unwrap().unwrap();
    assert_eq!(released.id, job);
    assert_eq!(released.policy, "keep");
    assert_eq!(db.release_waiting_jobs("keep").unwrap(), 0);
}

#[test]
fn waiting_job_blocks_profile_deletion() {
    let db = Db::open_in_memory().unwrap();
    db.save_profile(&profile("p", "P")).unwrap();
    db.save_profile(&profile("q", "Q")).unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let job = db
        .enqueue_job_with_status(photo, "p", "keep", QueueStatus::Waiting)
        .unwrap();

    assert_eq!(db.active_job_count_for_profile("p").unwrap(), 1);
    assert!(matches!(db.delete_profile("p"), Err(CoreError::Invalid(_))));

    db.update_job(&job, QueueStatus::Completed, 1.0, None, None)
        .unwrap();
    db.delete_profile("p").unwrap();
}

#[test]
fn retry_job_resets_failed_job_and_enforces_cap() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let job = db.enqueue_job(photo, "p", "keep").unwrap();
    db.next_pending_job().unwrap();
    db.update_job(&job, QueueStatus::Failed, 0.0, Some("boom"), None)
        .unwrap();

    for expected in 1..=MAX_JOB_RETRIES {
        assert!(db.retry_job(&job).unwrap());
        let row = &db.list_jobs(1).unwrap()[0];
        assert_eq!(row.status, QueueStatus::Pending);
        assert_eq!(row.retry_count, expected);
        assert!(row.error.is_none());
        assert!(row.started_at.is_none());
        assert!(row.finished_at.is_none());
        db.next_pending_job().unwrap();
        db.update_job(&job, QueueStatus::Failed, 0.0, Some("boom"), None)
            .unwrap();
    }

    assert!(!db.retry_job(&job).unwrap());
    assert_eq!(db.list_jobs(1).unwrap()[0].retry_count, MAX_JOB_RETRIES);
}

#[test]
fn retry_resets_progress_and_rejects_non_failed_jobs() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let job = db.enqueue_job(photo, "p", "keep").unwrap();

    // A job that never failed cannot be retried, and neither can a
    // missing one.
    assert!(!db.retry_job(&job).unwrap());
    assert!(!db.retry_job("missing-job").unwrap());

    db.next_pending_job().unwrap();
    db.update_job(&job, QueueStatus::Failed, 0.5, Some("boom"), None)
        .unwrap();
    assert!(db.retry_job(&job).unwrap());
    let row = &db.list_jobs(1).unwrap()[0];
    assert_eq!(row.status, QueueStatus::Pending);
    assert_eq!(row.progress, 0.0);
    assert!(row.error.is_none());

    // The bulk retry resets progress the same way.
    db.next_pending_job().unwrap();
    db.update_job(&job, QueueStatus::Failed, 0.75, Some("boom again"), None)
        .unwrap();
    assert_eq!(db.retry_failed_jobs().unwrap(), 1);
    let row = &db.list_jobs(1).unwrap()[0];
    assert_eq!(row.status, QueueStatus::Pending);
    assert_eq!(row.progress, 0.0);
    assert!(row.error.is_none());
}

#[test]
fn resume_interrupted_compression_jobs_requeues_processing_without_retry() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let job = db.enqueue_job(photo, "p", "keep").unwrap();
    let claimed = db.next_pending_job().unwrap().unwrap();
    assert_eq!(claimed.id, job);
    assert_eq!(claimed.status, QueueStatus::Processing);

    assert_eq!(db.resume_interrupted_compression_jobs().unwrap(), 1);
    let resumed = &db.list_jobs(1).unwrap()[0];
    assert_eq!(resumed.status, QueueStatus::Pending);
    assert_eq!(resumed.retry_count, 0);
    assert_eq!(resumed.progress, 0.0);
    assert!(resumed.error.is_none());
    assert!(resumed.started_at.is_none());
    assert!(resumed.finished_at.is_none());

    // A second call is a no-op.
    assert_eq!(db.resume_interrupted_compression_jobs().unwrap(), 0);
}

#[test]
fn retry_failed_jobs_returns_only_eligible_rows() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let first = db.enqueue_job(photo, "p", "keep").unwrap();
    let capped = db.enqueue_job(photo, "p", "keep").unwrap();
    let completed = db.enqueue_job(photo, "p", "keep").unwrap();
    for job in [&first, &capped, &completed] {
        db.next_pending_job().unwrap();
        db.update_job(job, QueueStatus::Failed, 0.0, Some("boom"), None)
            .unwrap();
    }
    db.connection()
        .execute(
            "UPDATE compression_jobs SET retry_count = ?1 WHERE id = ?2",
            params![MAX_JOB_RETRIES, capped],
        )
        .unwrap();
    db.update_job(&completed, QueueStatus::Completed, 1.0, None, None)
        .unwrap();

    assert_eq!(db.retry_failed_jobs().unwrap(), 1);
    assert_eq!(db.count_jobs(QueueStatus::Pending).unwrap(), 1);
    assert_eq!(
        db.list_jobs(10)
            .unwrap()
            .into_iter()
            .find(|row| row.id == first)
            .unwrap()
            .retry_count,
        1
    );
}

#[test]
fn count_retryable_failed_jobs_skips_capped_and_non_failed() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let first = db.enqueue_job(photo, "p", "keep").unwrap();
    let second = db.enqueue_job(photo, "p", "keep").unwrap();
    let capped = db.enqueue_job(photo, "p", "keep").unwrap();
    let completed = db.enqueue_job(photo, "p", "keep").unwrap();
    for job in [&first, &second, &capped, &completed] {
        db.next_pending_job().unwrap();
        db.update_job(job, QueueStatus::Failed, 0.0, Some("boom"), None)
            .unwrap();
    }
    db.connection()
        .execute(
            "UPDATE compression_jobs SET retry_count = ?1 WHERE id = ?2",
            params![MAX_JOB_RETRIES, capped],
        )
        .unwrap();
    db.update_job(&completed, QueueStatus::Completed, 1.0, None, None)
        .unwrap();

    assert_eq!(db.count_retryable_failed_jobs().unwrap(), 2);
    assert_eq!(db.retry_failed_jobs().unwrap(), 2);
    assert_eq!(db.count_retryable_failed_jobs().unwrap(), 0);
}

#[test]
fn list_jobs_by_status_filters_and_limits() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let failed = db.enqueue_job(photo, "p", "keep").unwrap();
    db.enqueue_job(photo, "p", "keep").unwrap();
    db.update_job(&failed, QueueStatus::Failed, 0.0, Some("boom"), None)
        .unwrap();

    let rows = db.list_jobs_by_status(QueueStatus::Failed, 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, failed);
    assert!(
        db.list_jobs_by_status(QueueStatus::Pending, 10)
            .unwrap()
            .len()
            == 1
    );
}

#[test]
fn list_jobs_by_status_clamps_non_positive_limits() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let first = db.enqueue_job(photo, "p", "keep").unwrap();
    let second = db.enqueue_job(photo, "p", "keep").unwrap();
    for job in [&first, &second] {
        db.update_job(job, QueueStatus::Failed, 0.0, Some("boom"), None)
            .unwrap();
    }

    // The documented contract: the limit never drops below 1.
    assert_eq!(
        db.list_jobs_by_status(QueueStatus::Failed, 0)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.list_jobs_by_status(QueueStatus::Failed, -5)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.list_jobs_by_status(QueueStatus::Failed, 2)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn daemon_error_upsert_increments_attempts_and_clear_removes_it() {
    let db = Db::open_in_memory().unwrap();
    db.record_daemon_error(DaemonOperation::Scan, "cannot scan", Some("/missing"))
        .unwrap();
    db.record_daemon_error(DaemonOperation::Scan, "still missing", None)
        .unwrap();

    let errors = db.daemon_errors().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].operation, DaemonOperation::Scan);
    assert_eq!(errors[0].message, "still missing");
    assert_eq!(errors[0].detail, None);
    assert_eq!(errors[0].attempts, 2);

    db.clear_daemon_error(DaemonOperation::Scan).unwrap();
    assert!(db.daemon_errors().unwrap().is_empty());
}

#[test]
fn keyed_daemon_errors_round_trip() {
    let db = Db::open_in_memory().unwrap();
    db.record_daemon_error_i18n(
        DaemonOperation::Scan,
        "Photo scan failed",
        Some("daemon.scan_failed"),
        Some(r#"{"count":"3"}"#),
        Some("detail"),
    )
    .unwrap();

    let errors = db.daemon_errors().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].message_key.as_deref(), Some("daemon.scan_failed"));
    assert_eq!(errors[0].message_args.as_deref(), Some(r#"{"count":"3"}"#));
}

#[test]
fn legacy_daemon_errors_have_no_key_or_args() {
    let db = Db::open_in_memory().unwrap();
    db.record_daemon_error(DaemonOperation::Log, "log failed", None)
        .unwrap();

    let errors = db.daemon_errors().unwrap();
    assert!(errors[0].message_key.is_none());
    assert!(errors[0].message_args.is_none());
}

#[test]
fn photo_context_session_id_has_an_index() {
    let db = Db::open_in_memory().unwrap();
    let version: i64 = db
        .connection()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);

    let sql: Option<String> = db
        .connection()
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'index' \
                 AND name = 'idx_context_session'",
            [],
            |row| row.get(0),
        )
        .ok();
    let sql = sql.expect("photo_context.session_id index");
    assert!(
        sql.to_lowercase().contains("on photo_context(session_id)"),
        "{sql}"
    );
}

#[test]
fn record_link_attempt_follows_the_backoff_schedule() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    for (attempt, expected) in [
        (1, 1_000 + 3_600),
        (2, 1_000 + 6 * 3_600),
        (3, 1_000 + 24 * 3_600),
        (4, 1_000 + 7 * 24 * 3_600),
        (5, 1_000 + 7 * 24 * 3_600),
    ] {
        db.record_link_attempt(photo, 1_000).unwrap();
        let (attempts, next_retry): (i64, Option<i64>) = db
            .conn
            .query_row(
                "SELECT link_attempts, link_next_retry_at FROM photos WHERE id = ?1",
                params![photo],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(attempts, attempt);
        assert_eq!(next_retry, Some(expected));
    }
}

#[test]
fn set_photo_context_clears_link_retry_state() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    db.record_link_attempt(photo, 1_000).unwrap();

    db.set_photo_context(&PhotoContext {
        photo_id: photo,
        session_id: None,
        world_id: None,
        world_name: Some("Linked World".to_string()),
        instance_id: None,
        link_method: "log".to_string(),
        confidence: 0.9,
        author: None,
        linked_at: "2026-11-06T00:00:00+00:00".to_string(),
    })
    .unwrap();

    let (attempts, next_retry): (i64, Option<i64>) = db
        .conn
        .query_row(
            "SELECT link_attempts, link_next_retry_at FROM photos WHERE id = ?1",
            params![photo],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(attempts, 0);
    assert_eq!(next_retry, None);
}

#[test]
fn unlinked_photos_due_respects_schedule_and_visibility() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let due = db
        .upsert_photo(&sample_photo(root, "/photos/due.png", 300))
        .unwrap();
    let future = db
        .upsert_photo(&sample_photo(root, "/photos/future.png", 200))
        .unwrap();
    let linked = db
        .upsert_photo(&sample_photo(root, "/photos/linked.png", 100))
        .unwrap();
    let missing = db
        .upsert_photo(&sample_photo(root, "/photos/missing.png", 50))
        .unwrap();
    db.record_link_attempt(future, 1_000).unwrap();
    db.set_photo_context(&PhotoContext {
        photo_id: linked,
        session_id: None,
        world_id: None,
        world_name: None,
        instance_id: None,
        link_method: "log".to_string(),
        confidence: 0.9,
        author: None,
        linked_at: "2026-11-06T00:00:00+00:00".to_string(),
    })
    .unwrap();
    db.mark_missing(missing).unwrap();

    let ids = |photos: Vec<Photo>| photos.into_iter().map(|photo| photo.id).collect::<Vec<_>>();
    assert_eq!(ids(db.unlinked_photos_due(10, 1_000).unwrap()), vec![due]);
    assert_eq!(
        ids(db.unlinked_photos_due(10, 1_000 + 3_600).unwrap()),
        vec![due, future]
    );
}

#[test]
fn unlinked_photos_due_orders_least_attempted_then_oldest() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let newest = db
        .upsert_photo(&sample_photo(root, "/photos/newest.png", 400))
        .unwrap();
    let middle = db
        .upsert_photo(&sample_photo(root, "/photos/middle.png", 300))
        .unwrap();
    let older = db
        .upsert_photo(&sample_photo(root, "/photos/older.png", 200))
        .unwrap();
    let oldest = db
        .upsert_photo(&sample_photo(root, "/photos/oldest.png", 100))
        .unwrap();
    db.record_link_attempt(older, 0).unwrap();
    db.record_link_attempt(oldest, 0).unwrap();
    db.record_link_attempt(oldest, 0).unwrap();

    let now = 1_000_000;
    let ids = |photos: Vec<Photo>| photos.into_iter().map(|photo| photo.id).collect::<Vec<_>>();
    // Attempts first, then capture time; the two zero-attempt photos are
    // oldest-first so the oldest unlinked photo is not starved.
    assert_eq!(
        ids(db.unlinked_photos_due(10, now).unwrap()),
        vec![middle, newest, older, oldest]
    );
    assert_eq!(
        ids(db.unlinked_photos_due(2, now).unwrap()),
        vec![middle, newest]
    );

    // The first page is now back in backoff, so the next batch reaches the
    // older photos instead of returning the same two rows forever.
    db.record_link_attempt(middle, now).unwrap();
    db.record_link_attempt(newest, now).unwrap();
    assert_eq!(
        ids(db.unlinked_photos_due(2, now).unwrap()),
        vec![older, oldest]
    );
}

#[test]
fn make_unlinked_photos_due_clears_only_unlinked_photos() {
    let db = Db::open_in_memory().unwrap();
    let unlinked = seed_photo(&db);
    let linked = seed_other_photo(&db);
    db.record_link_attempt(unlinked, 1_000).unwrap();
    db.set_photo_context(&PhotoContext {
        photo_id: linked,
        session_id: None,
        world_id: None,
        world_name: None,
        instance_id: None,
        link_method: "log".to_string(),
        confidence: 0.9,
        author: None,
        linked_at: "2026-11-06T00:00:00+00:00".to_string(),
    })
    .unwrap();
    db.conn
        .execute(
            "UPDATE photos SET link_next_retry_at = 99_999 WHERE id = ?1",
            params![linked],
        )
        .unwrap();

    assert_eq!(db.make_unlinked_photos_due().unwrap(), 1);

    let next_retry: Option<i64> = db
        .conn
        .query_row(
            "SELECT link_next_retry_at FROM photos WHERE id = ?1",
            params![unlinked],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(next_retry, None);
    let attempts: i64 = db
        .conn
        .query_row(
            "SELECT link_attempts FROM photos WHERE id = ?1",
            params![unlinked],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(attempts, 1);
    let linked_retry: Option<i64> = db
        .conn
        .query_row(
            "SELECT link_next_retry_at FROM photos WHERE id = ?1",
            params![linked],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(linked_retry, Some(99_999));
}

#[test]
fn open_rejects_a_database_from_a_newer_schema_version() {
    let newer = SCHEMA_VERSION + 1;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let db = Db::open(&path).unwrap();
    db.connection()
        .pragma_update(None, "user_version", newer)
        .unwrap();
    drop(db);

    match Db::open(&path) {
        Err(CoreError::Invalid(message)) => {
            assert!(message.contains(&newer.to_string()), "{message}");
            assert!(message.contains(&SCHEMA_VERSION.to_string()), "{message}");
        }
        Err(error) => panic!("expected a typed future-version error, got {error}"),
        Ok(_) => panic!("expected Db::open to reject schema version {newer}"),
    }
    match Db::open_read_only(&path) {
        Err(CoreError::Invalid(message)) => {
            assert!(message.contains(&newer.to_string()), "{message}");
            assert!(message.contains(&SCHEMA_VERSION.to_string()), "{message}");
        }
        Err(error) => panic!("expected a typed future-version error, got {error}"),
        Ok(_) => panic!("expected Db::open_read_only to reject schema version {newer}"),
    }
}

#[test]
fn next_pending_job_query_uses_the_claim_index() {
    let db = Db::open_in_memory().unwrap();
    let mut stmt = db
        .conn
        .prepare_cached(
            "EXPLAIN QUERY PLAN SELECT id FROM compression_jobs \
                 WHERE status = 'pending' ORDER BY created_at, rowid LIMIT 1",
        )
        .unwrap();
    let plan = stmt
        .query_map([], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .join("\n");
    assert!(plan.contains("idx_jobs_claim"), "{plan}");
}

#[test]
fn sessions_near_query_uses_the_session_indexes() {
    let db = Db::open_in_memory().unwrap();
    let mut stmt = db
        .conn
        .prepare_cached(&format!("EXPLAIN QUERY PLAN {SESSIONS_NEAR_SQL}"))
        .unwrap();
    let plan = stmt
        .query_map(params![1000, 30], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
        .join("\n");
    assert!(plan.contains("idx_sessions_started"), "{plan}");
    assert!(plan.contains("idx_sessions_ended"), "{plan}");
}

#[test]
fn active_installation_is_unique_per_plugin() {
    let db = Db::open_in_memory().unwrap();
    db.upsert_plugin_installation(&ai_install(AI_PLUGIN, "linux-vulkan", "1.0.0", true))
        .unwrap();
    db.upsert_plugin_installation(&ai_install(AI_PLUGIN, "linux-vulkan", "2.0.0", false))
        .unwrap();
    assert_eq!(
        db.active_plugin_installation(AI_PLUGIN)
            .unwrap()
            .unwrap()
            .version,
        "1.0.0"
    );

    // Upserting another install as active must clear the previous one
    // inside the same transaction instead of violating the unique index.
    db.upsert_plugin_installation(&ai_install(AI_PLUGIN, "linux-vulkan", "2.0.0", true))
        .unwrap();
    let active = db.active_plugin_installation(AI_PLUGIN).unwrap().unwrap();
    assert_eq!(active.version, "2.0.0");
    assert_eq!(
        db.list_plugin_installations(AI_PLUGIN)
            .unwrap()
            .iter()
            .filter(|install| install.active)
            .count(),
        1
    );

    // Active is unique per plugin, not per variant.
    db.upsert_plugin_installation(&ai_install(AI_PLUGIN, "windows-vulkan", "1.0.0", true))
        .unwrap();
    let active = db.active_plugin_installation(AI_PLUGIN).unwrap().unwrap();
    assert_eq!(active.variant, "windows-vulkan");
    assert_eq!(
        db.list_plugin_installations(AI_PLUGIN)
            .unwrap()
            .iter()
            .filter(|install| install.active)
            .count(),
        1
    );
}

#[test]
fn plugin_installation_crud_roundtrip_and_active_selection() {
    let db = Db::open_in_memory().unwrap();
    for version in ["1.2.0", "1.9.9", "1.10.0"] {
        db.upsert_plugin_installation(&ai_install(AI_PLUGIN, "linux-vulkan", version, false))
            .unwrap();
    }
    let installations = db.list_plugin_installations(AI_PLUGIN).unwrap();
    assert_eq!(installations.len(), 3);
    let install = installations
        .iter()
        .find(|install| install.version == "1.10.0")
        .unwrap();
    assert_eq!(install.plugin_id, AI_PLUGIN);
    assert_eq!(install.variant, "linux-vulkan");
    assert_eq!(install.protocol_version, 1);
    assert_eq!(
        install.path,
        "/plugins/vrc-photon-autotag/linux-vulkan/1.10.0"
    );
    assert_eq!(install.manifest_json, r#"{"version":"1.10.0"}"#);
    assert_eq!(install.catalog_version.as_deref(), Some("2026-11-03.1"));
    assert!(db.active_plugin_installation(AI_PLUGIN).unwrap().is_none());

    // `None` picks the highest semantic version, not the lexicographic
    // maximum, for the requested variant.
    assert!(
        db.set_active_plugin_installation(AI_PLUGIN, "linux-vulkan", None)
            .unwrap()
    );
    let active = db.active_plugin_installation(AI_PLUGIN).unwrap().unwrap();
    assert_eq!(active.version, "1.10.0");

    assert!(
        db.set_active_plugin_installation(AI_PLUGIN, "linux-vulkan", Some("1.2.0"))
            .unwrap()
    );
    let active = db.active_plugin_installation(AI_PLUGIN).unwrap().unwrap();
    assert_eq!(active.version, "1.2.0");

    assert!(
        !db.set_active_plugin_installation(AI_PLUGIN, "linux-vulkan", Some("9.9.9"))
            .unwrap()
    );
    assert!(
        !db.set_active_plugin_installation(AI_PLUGIN, "windows-vulkan", None)
            .unwrap()
    );
    let active = db.active_plugin_installation(AI_PLUGIN).unwrap().unwrap();
    assert_eq!(active.version, "1.2.0");

    db.remove_plugin_installation(AI_PLUGIN, "linux-vulkan", "1.2.0")
        .unwrap();
    assert_eq!(db.list_plugin_installations(AI_PLUGIN).unwrap().len(), 2);
    assert!(db.active_plugin_installation(AI_PLUGIN).unwrap().is_none());
}

#[test]
fn plugin_state_upserts_model_and_probe() {
    let db = Db::open_in_memory().unwrap();
    assert!(db.plugin_state(AI_PLUGIN).unwrap().is_none());

    db.set_plugin_probe_json(AI_PLUGIN, Some(r#"{"backend":"cpu"}"#))
        .unwrap();
    let state = db.plugin_state(AI_PLUGIN).unwrap().unwrap();
    assert_eq!(state.plugin_id, AI_PLUGIN);
    assert_eq!(state.model_state, ModelState::Missing);
    assert_eq!(state.model_bytes_downloaded, 0);
    assert_eq!(state.model_bytes_total, None);
    assert_eq!(state.probe_json.as_deref(), Some(r#"{"backend":"cpu"}"#));

    db.set_plugin_model_state(
        AI_PLUGIN,
        ModelState::Downloading,
        Some("rev-1"),
        512,
        Some(2048),
        None,
    )
    .unwrap();
    let state = db.plugin_state(AI_PLUGIN).unwrap().unwrap();
    assert_eq!(state.model_state, ModelState::Downloading);
    assert_eq!(state.model_revision.as_deref(), Some("rev-1"));
    assert_eq!(state.model_bytes_downloaded, 512);
    assert_eq!(state.model_bytes_total, Some(2048));
    assert_eq!(state.model_error, None);
    assert_eq!(state.probe_json.as_deref(), Some(r#"{"backend":"cpu"}"#));

    db.set_plugin_model_state(
        AI_PLUGIN,
        ModelState::Error,
        Some("rev-1"),
        512,
        Some(2048),
        Some("checksum mismatch"),
    )
    .unwrap();
    let state = db.plugin_state(AI_PLUGIN).unwrap().unwrap();
    assert_eq!(state.model_state, ModelState::Error);
    assert_eq!(state.model_error.as_deref(), Some("checksum mismatch"));

    // The model-state insert path must work without a prior probe row.
    db.set_plugin_model_state("other-plugin", ModelState::Ready, None, 0, None, None)
        .unwrap();
    let state = db.plugin_state("other-plugin").unwrap().unwrap();
    assert_eq!(state.model_state, ModelState::Ready);
    assert_eq!(state.probe_json, None);

    db.set_plugin_probe_json(AI_PLUGIN, None).unwrap();
    let state = db.plugin_state(AI_PLUGIN).unwrap().unwrap();
    assert_eq!(state.probe_json, None);
    assert_eq!(state.model_state, ModelState::Error);
}

#[test]
fn catalog_sync_preserves_user_thresholds() {
    let db = Db::open_in_memory().unwrap();
    db.sync_ai_catalog(AI_PLUGIN, &ai_catalog("2026-11-03.1", 0.5, 0.5))
        .unwrap();
    let definitions = db.ai_tag_definitions(AI_PLUGIN).unwrap();
    assert_eq!(definitions.len(), 3);
    let sea = definitions
        .iter()
        .find(|definition| definition.tag_key == "sea")
        .unwrap();
    assert_eq!(sea.branch, "outdoor");
    assert_eq!(sea.sort_order, 0);
    assert_eq!(sea.default_threshold, 0.5);
    assert_eq!(sea.name_en.as_deref(), Some("Sea"));
    let room = definitions
        .iter()
        .find(|definition| definition.tag_key == "japanese_room")
        .unwrap();
    assert_eq!(room.branch, "indoor");
    assert_eq!(room.sort_order, 0);

    db.set_ai_tag_threshold(AI_PLUGIN, "sea", 0.8).unwrap();
    let photo = seed_photo(&db);
    seed_ai_tag(&db, photo, AI_PLUGIN, "sea", 0.75, true);

    db.sync_ai_catalog(AI_PLUGIN, &ai_catalog("2026-11-04.1", 0.7, 0.5))
        .unwrap();
    let definitions = db.ai_tag_definitions(AI_PLUGIN).unwrap();
    let sea = definitions
        .iter()
        .find(|definition| definition.tag_key == "sea")
        .unwrap();
    assert_eq!(sea.default_threshold, 0.7);
    assert_eq!(sea.catalog_version, "2026-11-04.1");

    let thresholds = db.ai_tag_thresholds(AI_PLUGIN).unwrap();
    assert_eq!(thresholds.len(), 1);
    assert_eq!(thresholds[0].tag_key, "sea");
    assert_eq!(thresholds[0].threshold, 0.8);

    // The user override (0.8) wins over the new default (0.7).
    let applied: i64 = db
        .connection()
        .query_row(
            "SELECT applied FROM photo_ai_tags \
                 WHERE photo_id = ?1 AND plugin_id = ?2 AND tag_key = 'sea'",
            params![photo, AI_PLUGIN],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(applied, 0);
}

#[test]
fn catalog_sync_updates_defaults_and_reapplies() {
    let db = Db::open_in_memory().unwrap();
    db.sync_ai_catalog(AI_PLUGIN, &ai_catalog("2026-11-03.1", 0.5, 0.5))
        .unwrap();
    let photo = seed_photo(&db);
    seed_ai_tag(&db, photo, AI_PLUGIN, "sea", 0.6, true);
    seed_ai_tag(&db, photo, AI_PLUGIN, "park", 0.8, true);

    db.sync_ai_catalog(AI_PLUGIN, &ai_catalog("2026-11-04.1", 0.7, 0.5))
        .unwrap();

    let definitions = db.ai_tag_definitions(AI_PLUGIN).unwrap();
    let sea = definitions
        .iter()
        .find(|definition| definition.tag_key == "sea")
        .unwrap();
    assert_eq!(sea.default_threshold, 0.7);
    assert_eq!(sea.catalog_version, "2026-11-04.1");

    let applied = |tag_key: &str| -> i64 {
        db.connection()
            .query_row(
                "SELECT applied FROM photo_ai_tags \
                     WHERE photo_id = ?1 AND plugin_id = ?2 AND tag_key = ?3",
                params![photo, AI_PLUGIN, tag_key],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(applied("sea"), 0);
    assert_eq!(applied("park"), 1);
    assert!(db.ai_tag_thresholds(AI_PLUGIN).unwrap().is_empty());
}

#[test]
fn catalog_sync_stores_atmosphere_group_and_moves_scene_tags() {
    let db = Db::open_in_memory().unwrap();
    let mut old = ai_catalog("2026-11-03.1", 0.5, 0.5);
    old.outdoor.push(tag("sunset", "夕景", Some("Sunset"), 0.5));
    db.sync_ai_catalog(AI_PLUGIN, &old).unwrap();

    let new = ai_catalog_with_atmosphere("2026-11-07.1");
    db.sync_ai_catalog(AI_PLUGIN, &new).unwrap();
    let definitions = db.ai_tag_definitions(AI_PLUGIN).unwrap();
    let sunset = definitions
        .iter()
        .find(|definition| definition.tag_key == "sunset")
        .unwrap();
    assert_eq!(sunset.branch, "atmosphere");
    assert_eq!(sunset.tag_group, "scene");
    let clear = definitions
        .iter()
        .find(|definition| definition.tag_key == "weather_clear")
        .unwrap();
    assert_eq!(clear.branch, "atmosphere");
    assert_eq!(clear.tag_group, "weather");
}

#[test]
fn catalog_sync_does_not_enqueue_tagging_jobs() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    db.sync_ai_catalog(AI_PLUGIN, &ai_catalog_with_atmosphere("2026-11-07.1"))
        .unwrap();
    assert!(db.tagging_jobs_for_photo(photo).unwrap().is_empty());
    // The existing photo has no new atmosphere tag rows.
    assert!(db.photo_ai_tags(photo, false).unwrap().is_empty());
}

#[test]
fn reapply_thresholds_updates_applied() {
    let db = Db::open_in_memory().unwrap();
    db.sync_ai_catalog(AI_PLUGIN, &ai_catalog("2026-11-03.1", 0.5, 0.5))
        .unwrap();
    let photo = seed_photo(&db);
    seed_ai_tag(&db, photo, AI_PLUGIN, "sea", 0.4, true);
    seed_ai_tag(&db, photo, AI_PLUGIN, "japanese_room", 0.6, false);
    db.connection()
        .execute(
            "INSERT INTO ai_tag_thresholds(plugin_id, tag_key, threshold, updated_at) \
                 VALUES (?1, 'japanese_room', 0.7, ?2)",
            params![AI_PLUGIN, "2026-11-04T00:00:00+00:00"],
        )
        .unwrap();

    assert_eq!(db.reapply_ai_tag_thresholds(AI_PLUGIN).unwrap(), 2);
    let applied = |plugin_id: &str, tag_key: &str| -> i64 {
        db.connection()
            .query_row(
                "SELECT applied FROM photo_ai_tags \
                     WHERE photo_id = ?1 AND plugin_id = ?2 AND tag_key = ?3",
                params![photo, plugin_id, tag_key],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(applied(AI_PLUGIN, "sea"), 0);
    assert_eq!(applied(AI_PLUGIN, "japanese_room"), 0);

    seed_ai_tag(&db, photo, "other-plugin", "sea", 0.9, false);
    assert_eq!(db.reapply_ai_tag_thresholds(AI_PLUGIN).unwrap(), 2);
    assert_eq!(applied("other-plugin", "sea"), 0);
}

#[test]
fn ai_tag_threshold_crud() {
    let db = Db::open_in_memory().unwrap();
    assert!(db.ai_tag_thresholds(AI_PLUGIN).unwrap().is_empty());

    db.set_ai_tag_threshold(AI_PLUGIN, "sea", 0.8).unwrap();
    let thresholds = db.ai_tag_thresholds(AI_PLUGIN).unwrap();
    assert_eq!(thresholds.len(), 1);
    assert_eq!(thresholds[0].plugin_id, AI_PLUGIN);
    assert_eq!(thresholds[0].tag_key, "sea");
    assert_eq!(thresholds[0].threshold, 0.8);

    db.set_ai_tag_threshold(AI_PLUGIN, "sea", 0.6).unwrap();
    db.set_ai_tag_threshold(AI_PLUGIN, "park", 0.3).unwrap();
    let thresholds = db.ai_tag_thresholds(AI_PLUGIN).unwrap();
    assert_eq!(thresholds.len(), 2);
    assert_eq!(
        thresholds
            .iter()
            .find(|threshold| threshold.tag_key == "sea")
            .unwrap()
            .threshold,
        0.6
    );

    db.reset_ai_tag_threshold(AI_PLUGIN, "sea").unwrap();
    let thresholds = db.ai_tag_thresholds(AI_PLUGIN).unwrap();
    assert_eq!(thresholds.len(), 1);
    assert_eq!(thresholds[0].tag_key, "park");

    // Resetting a missing override is a no-op.
    db.reset_ai_tag_threshold(AI_PLUGIN, "sea").unwrap();
    assert_eq!(db.ai_tag_thresholds(AI_PLUGIN).unwrap().len(), 1);

    assert!(matches!(
        db.set_ai_tag_threshold(AI_PLUGIN, "sea", 1.5),
        Err(CoreError::Invalid(_))
    ));
    assert!(matches!(
        db.set_ai_tag_threshold(AI_PLUGIN, "sea", f32::NAN),
        Err(CoreError::Invalid(_))
    ));
}

#[test]
fn apply_ai_tag_thresholds_updates_resets_and_reapplies_once() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    install_test_catalog(&db);
    db.save_photo_ai(
        photo,
        AI_PLUGIN,
        &test_ai_result(&[("sea", 0.75), ("park", 0.75)]),
    )
    .unwrap();
    let ai_revision = db.ai_revision().unwrap();
    let library_revision = db.library_revision().unwrap();

    let updated = db
        .apply_ai_tag_thresholds(
            AI_PLUGIN,
            &[("sea".to_string(), Some(0.8)), ("park".to_string(), None)],
        )
        .unwrap();

    assert!(updated >= 2);
    let overrides = db.ai_tag_thresholds(AI_PLUGIN).unwrap();
    assert_eq!(overrides.len(), 1);
    assert_eq!(overrides[0].tag_key, "sea");
    let tags = db.photo_ai_tags(photo, false).unwrap();
    assert!(
        !tags
            .iter()
            .find(|tag| tag.tag_key == "sea")
            .unwrap()
            .applied
    );
    assert!(
        tags.iter()
            .find(|tag| tag.tag_key == "park")
            .unwrap()
            .applied
    );
    assert!(db.ai_revision().unwrap() > ai_revision);
    assert_eq!(db.library_revision().unwrap(), library_revision);
}

#[test]
fn apply_ai_tag_thresholds_rejects_out_of_range_values() {
    let db = Db::open_in_memory().unwrap();
    assert!(
        db.apply_ai_tag_thresholds(AI_PLUGIN, &[("sea".to_string(), Some(1.1))])
            .is_err()
    );
    assert!(
        db.apply_ai_tag_thresholds(AI_PLUGIN, &[("sea".to_string(), Some(f32::NAN))])
            .is_err()
    );
}

#[test]
fn ai_result_bumps_ai_revision_without_moving_library_revision() {
    let db = Db::open_in_memory().unwrap();
    install_test_catalog(&db);
    let photo = seed_photo(&db);
    let library_before = db.library_revision().unwrap();
    let ai_before = db.ai_revision().unwrap();

    db.save_photo_ai(photo, AI_PLUGIN, &test_ai_result(&[("sea", 0.97)]))
        .unwrap();

    assert!(db.ai_revision().unwrap() > ai_before);
    assert_eq!(db.library_revision().unwrap(), library_before);
}

#[test]
fn threshold_reapplication_bumps_ai_revision() {
    let db = Db::open_in_memory().unwrap();
    db.sync_ai_catalog(AI_PLUGIN, &ai_catalog("2026-11-03.1", 0.5, 0.5))
        .unwrap();
    let photo = seed_photo(&db);
    db.save_photo_ai(photo, AI_PLUGIN, &test_ai_result(&[("sea", 0.6)]))
        .unwrap();

    let ai_before = db.ai_revision().unwrap();
    let library_before = db.library_revision().unwrap();
    db.set_ai_tag_threshold(AI_PLUGIN, "sea", 0.7).unwrap();
    assert!(!db.photo_ai_tags(photo, false).unwrap()[0].applied);
    assert!(db.ai_revision().unwrap() > ai_before);
    assert_eq!(db.library_revision().unwrap(), library_before);

    let ai_before = db.ai_revision().unwrap();
    let library_before = db.library_revision().unwrap();
    db.reset_ai_tag_threshold(AI_PLUGIN, "sea").unwrap();
    assert!(db.photo_ai_tags(photo, false).unwrap()[0].applied);
    assert!(db.ai_revision().unwrap() > ai_before);
    assert_eq!(db.library_revision().unwrap(), library_before);

    let ai_before = db.ai_revision().unwrap();
    let library_before = db.library_revision().unwrap();
    db.sync_ai_catalog(AI_PLUGIN, &ai_catalog("2026-11-04.1", 0.7, 0.5))
        .unwrap();
    assert!(!db.photo_ai_tags(photo, false).unwrap()[0].applied);
    assert!(db.ai_revision().unwrap() > ai_before);
    assert_eq!(db.library_revision().unwrap(), library_before);

    // A catalog refresh that keeps the defaults does not reapply.
    let ai_before = db.ai_revision().unwrap();
    let library_before = db.library_revision().unwrap();
    db.sync_ai_catalog(AI_PLUGIN, &ai_catalog("2026-11-05.1", 0.7, 0.5))
        .unwrap();
    assert_eq!(db.ai_revision().unwrap(), ai_before);
    assert_eq!(db.library_revision().unwrap(), library_before);
}

fn install_test_catalog(db: &Db) {
    db.sync_ai_catalog(AI_PLUGIN, &ai_catalog("2026-11-03.1", 0.5, 0.5))
        .unwrap();
}

fn test_ai_result(tags: &[(&str, f32)]) -> NewPhotoAi {
    NewPhotoAi {
        variant: "linux-cpu".to_string(),
        sidecar_version: "1.0.0".to_string(),
        model_version: "d1-2B-Q8_0".to_string(),
        model_revision: "rev-1".to_string(),
        catalog_version: "2026-11-03.1".to_string(),
        source: "original".to_string(),
        presence_threshold: 0.9,
        character_present: true,
        presence_probability: 0.99,
        people_min: Some(2),
        people_max: Some(3),
        people_median: Some(3),
        place_branch: Some("outdoor".to_string()),
        place_probabilities_json: Some(r#"{"indoor":0.1,"outdoor":0.8,"other":0.1}"#.to_string()),
        tags: tags
            .iter()
            .map(|(key, probability)| ((*key).to_string(), *probability))
            .collect(),
    }
}

fn seed_photos(db: &Db, count: usize) -> Vec<i64> {
    let root = db.add_root("/photos", "reference").unwrap();
    (0..count)
        .map(|index| {
            db.upsert_photo(&sample_photo(
                root,
                &format!("/photos/bulk-{index}.png"),
                1_700_000_000 + index as i64,
            ))
            .unwrap()
        })
        .collect()
}

fn claim_tagging_job(db: &Db, photo_id: i64) -> TaggingJob {
    assert_eq!(
        db.enqueue_tagging_jobs(&[photo_id], AI_PLUGIN, TaggingJobStatus::Pending)
            .unwrap(),
        1
    );
    let job = db.next_pending_tagging_job().unwrap().unwrap();
    assert_eq!(job.photo_id, photo_id);
    job
}

fn query_plan(db: &Db, sql: &str) -> Vec<String> {
    query_plan_with_values(db, sql, Vec::new())
}

fn query_plan_with_values(db: &Db, sql: &str, values: Vec<Value>) -> Vec<String> {
    let mut stmt = db
        .connection()
        .prepare_cached(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap();
    let rows = stmt
        .query_map(params_from_iter(values), |row| row.get::<_, String>(3))
        .unwrap();
    rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
}

/// Asserts the shared limit contract against a three-row fixture.
fn assert_limit_contract(label: &str, mut count: impl FnMut(i64) -> usize) {
    assert_eq!(count(0), 1, "{label}: zero must clamp to one");
    assert_eq!(count(-7), 1, "{label}: negative must clamp to one");
    assert_eq!(count(2), 2, "{label}: an in-range limit is honored");
    assert_eq!(
        count(i64::MAX),
        3,
        "{label}: an oversized limit must not hide rows"
    );
}

#[test]
fn ai_tags_never_touch_manual_tags() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    db.set_photo_tags(photo, &["manual".to_string()]).unwrap();
    install_test_catalog(&db);

    let result = NewPhotoAi {
        variant: "linux-cpu".into(),
        sidecar_version: "1.0.0".into(),
        model_version: "d1-2B".into(),
        model_revision: "rev".into(),
        catalog_version: "2026-11-03.1".into(),
        source: "original".into(),
        presence_threshold: 0.9,
        character_present: true,
        presence_probability: 0.99,
        people_min: Some(2),
        people_max: Some(3),
        people_median: Some(3),
        place_branch: Some("outdoor".into()),
        place_probabilities_json: Some(r#"{"indoor":0.1,"outdoor":0.8,"other":0.1}"#.into()),
        tags: vec![("sea".into(), 0.97), ("park".into(), 0.4)],
    };
    assert!(
        db.save_photo_ai(photo, "vrc-photon-autotag", &result)
            .unwrap()
    );
    assert_eq!(db.photo_tags(photo).unwrap(), vec!["manual".to_string()]);
    let tags = db.photo_ai_tags(photo, false).unwrap();
    assert_eq!(tags.len(), 2);
    assert!(
        tags.iter()
            .find(|tag| tag.tag_key == "sea")
            .unwrap()
            .applied
    );
    assert!(
        !tags
            .iter()
            .find(|tag| tag.tag_key == "park")
            .unwrap()
            .applied
    );
}

#[test]
fn ai_tag_filter_matches_only_applied_tags_of_the_ai_plugin() {
    let db = Db::open_in_memory().unwrap();
    let sea = seed_photo(&db);
    install_test_catalog(&db);
    db.save_photo_ai(
        sea,
        AI_PLUGIN,
        &test_ai_result(&[("sea", 0.97), ("park", 0.42)]),
    )
    .unwrap();
    let other = seed_other_photo(&db);
    seed_ai_tag(&db, other, "other-plugin", "sea", 0.99, true);

    let sea_filter = PhotoTextFilter {
        ai: AiPhotoFilter {
            tags: vec!["sea".into()],
            people_count: None,
        },
        ..Default::default()
    };
    assert_eq!(
        db.photo_index_text(&sea_filter)
            .unwrap()
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        vec![sea]
    );

    let unapplied = PhotoTextFilter {
        ai: AiPhotoFilter {
            tags: vec!["park".into()],
            people_count: None,
        },
        ..Default::default()
    };
    assert!(db.photo_index_text(&unapplied).unwrap().is_empty());
}

#[test]
fn people_count_filter_matches_inclusive_intervals() {
    let db = Db::open_in_memory().unwrap();
    let range = seed_photo(&db);
    install_test_catalog(&db);
    let mut range_result = test_ai_result(&[]);
    range_result.people_min = Some(2);
    range_result.people_max = Some(4);
    db.save_photo_ai(range, AI_PLUGIN, &range_result).unwrap();

    let zero = seed_other_photo(&db);
    let mut zero_result = test_ai_result(&[]);
    zero_result.people_min = Some(0);
    zero_result.people_max = Some(0);
    db.save_photo_ai(zero, AI_PLUGIN, &zero_result).unwrap();

    let no_ai = seed_photos(&db, 1)[0];
    for (count, expected) in [
        (0, vec![zero]),
        (2, vec![range]),
        (3, vec![range]),
        (4, vec![range]),
        (5, vec![]),
    ] {
        let filter = PhotoTextFilter {
            ai: AiPhotoFilter {
                tags: Vec::new(),
                people_count: Some(count),
            },
            ..Default::default()
        };
        let mut ids = db
            .photo_index_text(&filter)
            .unwrap()
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        assert_eq!(ids, expected, "count={count}");
    }

    let zero_filter = PhotoTextFilter {
        ai: AiPhotoFilter {
            tags: Vec::new(),
            people_count: Some(0),
        },
        ..Default::default()
    };
    assert!(
        db.photo_index_text(&zero_filter)
            .unwrap()
            .iter()
            .all(|entry| entry.id != no_ai)
    );
}

#[test]
fn photo_filter_ai_conditions_apply_to_search_photos_and_count() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    install_test_catalog(&db);
    db.save_photo_ai(photo, AI_PLUGIN, &test_ai_result(&[("sea", 0.9)]))
        .unwrap();
    seed_other_photo(&db);

    let filter = PhotoFilter {
        ai: AiPhotoFilter {
            tags: vec!["sea".into()],
            people_count: Some(3),
        },
        ..Default::default()
    };
    assert_eq!(db.count_search(&filter).unwrap(), 1);
    assert_eq!(
        db.search_photos(&filter)
            .unwrap()
            .iter()
            .map(|photo| photo.id)
            .collect::<Vec<_>>(),
        vec![photo]
    );
}

#[test]
fn ai_filters_use_the_ai_indexes() {
    let db = Db::open_in_memory().unwrap();
    install_test_catalog(&db);
    let photos = seed_photos(&db, 500);
    for (index, photo_id) in photos.iter().enumerate() {
        // sea は 10 枚に 1 枚だけにして、インデックスが選択されるようにする。
        let tag = if index % 10 == 0 { "sea" } else { "park" };
        let mut result = test_ai_result(&[(tag, 0.9)]);
        result.people_min = Some((index % 5) as u32);
        result.people_max = Some(((index % 5) + 2) as u32);
        db.save_photo_ai(*photo_id, AI_PLUGIN, &result).unwrap();
    }
    db.connection().execute("ANALYZE", []).unwrap();

    let filter = PhotoTextFilter {
        ai: AiPhotoFilter {
            tags: vec!["sea".into()],
            people_count: Some(3),
        },
        ..Default::default()
    };
    let (where_sql, values) = build_text_filter(&filter);
    let plan = query_plan_with_values(
        &db,
        &format!("SELECT p.id {PHOTO_FROM} WHERE {where_sql}"),
        values,
    );
    assert!(
        plan.iter()
            .any(|line| line.contains("idx_photo_ai_tags_applied")),
        "plan: {plan:#?}"
    );
    // `photo_ai` is looked up by its INTEGER PRIMARY KEY (photo_id) for the
    // correlated EXISTS, so SQLite does not need idx_photo_ai_people here.
    assert!(
        plan.iter()
            .any(|line| line.contains("SEARCH pa") && line.contains("INTEGER PRIMARY KEY")),
        "plan: {plan:#?}"
    );
}

#[test]
fn save_photo_ai_skips_missing_or_deleting_photos() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    install_test_catalog(&db);
    let result = test_ai_result(&[("sea", 0.97)]);
    let library_before = db.library_revision().unwrap();
    let ai_before = db.ai_revision().unwrap();

    db.connection()
        .execute(
            "UPDATE photos SET status = 'missing' WHERE id = ?1",
            params![photo],
        )
        .unwrap();
    assert!(!db.save_photo_ai(photo, AI_PLUGIN, &result).unwrap());
    assert!(db.photo_ai(photo).unwrap().is_none());
    assert!(db.photo_ai_tags(photo, false).unwrap().is_empty());

    db.connection()
        .execute(
            "UPDATE photos SET status = 'deleting' WHERE id = ?1",
            params![photo],
        )
        .unwrap();
    assert!(!db.save_photo_ai(photo, AI_PLUGIN, &result).unwrap());
    assert!(db.photo_ai(photo).unwrap().is_none());
    assert_eq!(db.ai_revision().unwrap(), ai_before);

    db.connection()
        .execute(
            "UPDATE photos SET status = 'present' WHERE id = ?1",
            params![photo],
        )
        .unwrap();
    assert!(db.save_photo_ai(photo, AI_PLUGIN, &result).unwrap());
    assert!(db.ai_revision().unwrap() > ai_before);
    assert_eq!(db.library_revision().unwrap(), library_before);
}

#[test]
fn saving_ai_result_replaces_the_previous_branch_tags() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    install_test_catalog(&db);

    assert!(
        db.save_photo_ai(
            photo,
            AI_PLUGIN,
            &test_ai_result(&[("sea", 0.9), ("park", 0.8)])
        )
        .unwrap()
    );
    assert_eq!(db.photo_ai_tags(photo, false).unwrap().len(), 2);

    let mut indoor = test_ai_result(&[("japanese_room", 0.6)]);
    indoor.place_branch = Some("indoor".to_string());
    assert!(db.save_photo_ai(photo, AI_PLUGIN, &indoor).unwrap());

    let tags = db.photo_ai_tags(photo, false).unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].tag_key, "japanese_room");
    let ai = db.photo_ai(photo).unwrap().unwrap();
    assert_eq!(ai.place_branch.as_deref(), Some("indoor"));
}

#[test]
fn saved_ai_tags_follow_threshold_changes() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    install_test_catalog(&db);
    db.save_photo_ai(
        photo,
        AI_PLUGIN,
        &test_ai_result(&[("sea", 0.6), ("park", 0.4)]),
    )
    .unwrap();

    let applied = |tag_key: &str| -> bool {
        db.photo_ai_tags(photo, false)
            .unwrap()
            .into_iter()
            .find(|tag| tag.tag_key == tag_key)
            .unwrap()
            .applied
    };
    assert!(applied("sea"));
    assert!(!applied("park"));

    db.set_ai_tag_threshold(AI_PLUGIN, "sea", 0.7).unwrap();
    assert!(!applied("sea"));
    db.reset_ai_tag_threshold(AI_PLUGIN, "sea").unwrap();
    assert!(applied("sea"));
}

#[test]
fn photo_ai_tags_with_names_join_the_catalog() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    install_test_catalog(&db);
    db.save_photo_ai(
        photo,
        AI_PLUGIN,
        &test_ai_result(&[("sea", 0.97), ("park", 0.4)]),
    )
    .unwrap();

    let views = db.photo_ai_tags_with_names(photo).unwrap();
    assert_eq!(views.len(), 2);
    let sea = views.iter().find(|view| view.tag_key == "sea").unwrap();
    assert_eq!(sea.name, "海");
    assert_eq!(sea.name_en.as_deref(), Some("Sea"));
    assert_eq!(sea.branch, "outdoor");
    assert_eq!(sea.tag_group, "");
    assert!((sea.probability - 0.97).abs() < f32::EPSILON);
    assert!(sea.applied);
    let park = views.iter().find(|view| view.tag_key == "park").unwrap();
    assert_eq!(park.name, "公園");
    assert_eq!(park.name_en, None);
    assert!(!park.applied);
}

#[test]
fn photo_ai_tags_with_names_include_atmosphere_branch_and_group() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    db.sync_ai_catalog(AI_PLUGIN, &ai_catalog_with_atmosphere("2026-11-07.1"))
        .unwrap();
    db.save_photo_ai(photo, AI_PLUGIN, &test_ai_result(&[("weather_clear", 0.8)]))
        .unwrap();

    let views = db.photo_ai_tags_with_names(photo).unwrap();
    let clear = views
        .iter()
        .find(|view| view.tag_key == "weather_clear")
        .unwrap();
    assert_eq!(clear.branch, "atmosphere");
    assert_eq!(clear.tag_group, "weather");
}

#[test]
fn ai_results_cascade_on_photo_delete() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    install_test_catalog(&db);
    db.save_photo_ai(photo, AI_PLUGIN, &test_ai_result(&[("sea", 0.97)]))
        .unwrap();
    assert!(db.photo_ai(photo).unwrap().is_some());
    assert_eq!(db.photo_ai_tags(photo, false).unwrap().len(), 1);

    db.connection()
        .execute("DELETE FROM photos WHERE id = ?1", params![photo])
        .unwrap();
    assert!(db.photo_ai(photo).unwrap().is_none());
    assert!(db.photo_ai_tags(photo, false).unwrap().is_empty());
}

#[test]
fn tagging_jobs_dedupe_active_work_and_respect_waiting() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    assert_eq!(
        db.enqueue_tagging_jobs(&[photo], AI_PLUGIN, TaggingJobStatus::Pending)
            .unwrap(),
        1
    );
    assert_eq!(
        db.enqueue_tagging_jobs(&[photo], AI_PLUGIN, TaggingJobStatus::Pending)
            .unwrap(),
        0
    );
    assert_eq!(db.tagging_job_counts(AI_PLUGIN).unwrap().pending, 1);
    assert!(db.next_pending_tagging_job().unwrap().is_some());
    assert!(db.next_pending_tagging_job().unwrap().is_none());
}

#[test]
fn tagging_enqueue_rejects_terminal_statuses() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    for status in [
        TaggingJobStatus::Processing,
        TaggingJobStatus::Completed,
        TaggingJobStatus::Failed,
        TaggingJobStatus::Cancelled,
    ] {
        assert!(matches!(
            db.enqueue_tagging_jobs(&[photo], AI_PLUGIN, status),
            Err(CoreError::Invalid(_))
        ));
    }
}

#[test]
fn next_pending_tagging_job_claims_with_the_active_variant() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    db.enqueue_tagging_jobs(&[photo], AI_PLUGIN, TaggingJobStatus::Pending)
        .unwrap();
    let job = db.next_pending_tagging_job().unwrap().unwrap();
    assert_eq!(job.status, TaggingJobStatus::Processing);
    assert_eq!(job.variant, None);

    db.upsert_plugin_installation(&ai_install(AI_PLUGIN, "linux-vulkan", "1.0.0", true))
        .unwrap();
    let photo = seed_other_photo(&db);
    db.enqueue_tagging_jobs(&[photo], AI_PLUGIN, TaggingJobStatus::Pending)
        .unwrap();
    let job = db.next_pending_tagging_job().unwrap().unwrap();
    assert_eq!(job.variant.as_deref(), Some("linux-vulkan"));
}

#[test]
fn backlog_enqueue_is_idempotent_and_newest_first() {
    let db = Db::open_in_memory().unwrap();
    let photos = seed_photos(&db, 1000);

    assert_eq!(db.enqueue_tagging_backlog(AI_PLUGIN).unwrap(), 1000);
    assert_eq!(db.enqueue_tagging_backlog(AI_PLUGIN).unwrap(), 0);
    assert_eq!(db.tagging_job_counts(AI_PLUGIN).unwrap().waiting, 1000);
    assert!(db.next_pending_tagging_job().unwrap().is_none());

    assert_eq!(db.release_waiting_tagging_jobs(AI_PLUGIN).unwrap(), 1000);
    assert_eq!(db.enqueue_tagging_backlog(AI_PLUGIN).unwrap(), 0);
    let mut claimed = Vec::new();
    for _ in 0..3 {
        claimed.push(db.next_pending_tagging_job().unwrap().unwrap().photo_id);
    }
    assert_eq!(
        claimed,
        photos.iter().rev().take(3).copied().collect::<Vec<_>>(),
        "the newest photos are claimed first"
    );
}

#[test]
fn tagging_claim_and_active_queries_avoid_temp_btrees() {
    let db = Db::open_in_memory().unwrap();

    let claim = query_plan(
        &db,
        "SELECT id FROM tagging_jobs WHERE status = 'pending' \
             ORDER BY created_at, rowid LIMIT 1",
    );
    assert!(
        !claim.iter().any(|line| line.contains("TEMP B-TREE")),
        "claim query sorts the pending backlog: {claim:?}"
    );
    assert!(
        claim
            .iter()
            .any(|line| line.contains("idx_tagging_jobs_claim")),
        "claim query did not use idx_tagging_jobs_claim: {claim:?}"
    );

    let active = query_plan(
        &db,
        "SELECT id FROM tagging_jobs INDEXED BY idx_tagging_jobs_active_list \
             WHERE status IN ('waiting', 'pending', 'processing') \
             ORDER BY created_at DESC, id DESC LIMIT 10",
    );
    assert!(
        !active.iter().any(|line| line.contains("TEMP B-TREE")),
        "active query sorts the whole queue: {active:?}"
    );
    assert!(
        active
            .iter()
            .any(|line| line.contains("idx_tagging_jobs_active_list")),
        "active query did not use idx_tagging_jobs_active_list: {active:?}"
    );

    let counts = query_plan(
        &db,
        "SELECT status, COUNT(*) FROM tagging_jobs WHERE plugin_id = 'vrc-photon-autotag' \
             GROUP BY status",
    );
    assert!(
        counts
            .iter()
            .any(|line| line.contains("idx_tagging_jobs_plugin_status")),
        "status counts did not use idx_tagging_jobs_plugin_status: {counts:?}"
    );
}

#[test]
fn release_waiting_moves_only_waiting_jobs_for_the_plugin() {
    let db = Db::open_in_memory().unwrap();
    let waiting = seed_photo(&db);
    let pending = seed_other_photo(&db);
    let other_plugin = seed_photos(&db, 3)[2];
    db.enqueue_tagging_jobs(&[waiting], AI_PLUGIN, TaggingJobStatus::Waiting)
        .unwrap();
    db.enqueue_tagging_jobs(&[pending], AI_PLUGIN, TaggingJobStatus::Pending)
        .unwrap();
    db.enqueue_tagging_jobs(&[other_plugin], "other-plugin", TaggingJobStatus::Waiting)
        .unwrap();

    assert_eq!(db.release_waiting_tagging_jobs(AI_PLUGIN).unwrap(), 1);
    let counts = db.tagging_job_counts(AI_PLUGIN).unwrap();
    assert_eq!(counts.waiting, 0);
    assert_eq!(counts.pending, 2);
    assert_eq!(db.tagging_job_counts("other-plugin").unwrap().waiting, 1);
}

#[test]
fn ui_active_jobs_prioritize_processing_over_the_ready_queue() {
    let db = Db::open_in_memory().unwrap();
    let photos = seed_photos(&db, 25);
    assert_eq!(
        db.enqueue_tagging_jobs(&photos, AI_PLUGIN, TaggingJobStatus::Pending)
            .unwrap(),
        25
    );

    let mut claimed = Vec::new();
    for _ in 0..3 {
        claimed.push(db.next_pending_tagging_job().unwrap().unwrap());
    }

    let ui = db.active_tagging_jobs_for_ui(3).unwrap();
    assert_eq!(ui.len(), 3, "the three running jobs must fill the card");
    assert!(
        ui.iter()
            .all(|job| job.status == TaggingJobStatus::Processing)
    );
    let mut ui_ids: Vec<_> = ui.iter().map(|job| job.id.clone()).collect();
    let mut claimed_ids: Vec<_> = claimed.iter().map(|job| job.id.clone()).collect();
    ui_ids.sort();
    claimed_ids.sort();
    assert_eq!(ui_ids, claimed_ids);

    // A larger snapshot adds pending work after the running jobs, never
    // before them.
    let wider = db.active_tagging_jobs_for_ui(5).unwrap();
    assert_eq!(wider.len(), 5);
    assert!(
        wider[..3]
            .iter()
            .all(|job| job.status == TaggingJobStatus::Processing)
    );
    assert!(
        wider[3..]
            .iter()
            .all(|job| job.status == TaggingJobStatus::Pending)
    );
}

#[test]
fn retry_caps_at_two() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let job = claim_tagging_job(&db, photo);
    db.fail_tagging_job(&job.id, TaggingErrorCode::Internal)
        .unwrap();

    for expected_retry in 1..=MAX_TAGGING_RETRIES {
        assert!(db.retry_tagging_job(&job.id).unwrap());
        let retried = db.tagging_job(&job.id).unwrap().unwrap();
        assert_eq!(retried.status, TaggingJobStatus::Pending);
        assert_eq!(retried.retry_count, expected_retry);
        assert_eq!(retried.progress, 0.0);
        assert_eq!(retried.stage, None);
        assert_eq!(retried.started_at, None);
        assert_eq!(retried.error, None);

        let claimed = db.next_pending_tagging_job().unwrap().unwrap();
        assert_eq!(claimed.id, job.id);
        db.fail_tagging_job(&job.id, TaggingErrorCode::Internal)
            .unwrap();
    }

    assert!(!db.retry_tagging_job(&job.id).unwrap());
    let failed = db.tagging_job(&job.id).unwrap().unwrap();
    assert_eq!(failed.status, TaggingJobStatus::Failed);
    assert_eq!(failed.retry_count, MAX_TAGGING_RETRIES);
}

#[test]
fn retry_failed_job_refuses_when_a_newer_active_job_exists() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let job = claim_tagging_job(&db, photo);
    db.fail_tagging_job(&job.id, TaggingErrorCode::Internal)
        .unwrap();
    assert_eq!(
        db.enqueue_tagging_jobs(&[photo], AI_PLUGIN, TaggingJobStatus::Pending)
            .unwrap(),
        1
    );

    assert!(!db.retry_tagging_job(&job.id).unwrap());
    let failed = db.tagging_job(&job.id).unwrap().unwrap();
    assert_eq!(failed.status, TaggingJobStatus::Failed);
    assert_eq!(failed.retry_count, 0);
    assert_eq!(db.tagging_job_counts(AI_PLUGIN).unwrap().pending, 1);
}

#[test]
fn defer_tagging_job_waits_without_counting_a_retry() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let job = claim_tagging_job(&db, photo);
    db.update_tagging_progress(&job.id, 0.3, TaggingStage::PeopleCount)
        .unwrap();
    db.defer_tagging_job(&job.id, TaggingErrorCode::PluginNotReady)
        .unwrap();

    let deferred = db.tagging_job(&job.id).unwrap().unwrap();
    assert_eq!(deferred.status, TaggingJobStatus::Pending);
    assert_eq!(deferred.error.as_deref(), Some("plugin_not_ready"));
    assert_eq!(deferred.retry_count, 0);
    assert_eq!(deferred.progress, 0.0);
    assert_eq!(deferred.stage, None);
    assert_eq!(deferred.started_at, None);

    let claimed = db.next_pending_tagging_job().unwrap().unwrap();
    assert_eq!(claimed.status, TaggingJobStatus::Processing);
    assert_eq!(claimed.error, None);
}

#[test]
fn cancel_processing_blocks_late_write() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    install_test_catalog(&db);
    let job = claim_tagging_job(&db, photo);
    db.update_tagging_progress(&job.id, 0.7, TaggingStage::PlaceTags)
        .unwrap();
    assert!(db.cancel_tagging_job(&job.id).unwrap());
    let cancelled = db.tagging_job(&job.id).unwrap().unwrap();
    assert_eq!(cancelled.status, TaggingJobStatus::Cancelled);
    assert_eq!(cancelled.progress, 0.0);
    assert_eq!(cancelled.stage, None);
    assert!(!db.cancel_tagging_job(&job.id).unwrap());

    assert!(
        !db.save_photo_ai_for_job(&job.id, &test_ai_result(&[("sea", 0.97)]))
            .unwrap()
    );
    assert!(db.photo_ai(photo).unwrap().is_none());

    db.complete_tagging_job(&job.id).unwrap();
    db.fail_tagging_job(&job.id, TaggingErrorCode::Internal)
        .unwrap();
    assert_eq!(
        db.tagging_job(&job.id).unwrap().unwrap().status,
        TaggingJobStatus::Cancelled
    );
}

#[test]
fn save_photo_ai_for_job_requires_processing_status() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    install_test_catalog(&db);
    db.enqueue_tagging_jobs(&[photo], AI_PLUGIN, TaggingJobStatus::Pending)
        .unwrap();
    let job = db.tagging_jobs_for_photo(photo).unwrap()[0].id.clone();
    let result = test_ai_result(&[("sea", 0.97)]);
    let library_before = db.library_revision().unwrap();
    let ai_before = db.ai_revision().unwrap();

    assert!(!db.save_photo_ai_for_job(&job, &result).unwrap());
    assert!(db.photo_ai(photo).unwrap().is_none());
    assert!(!db.save_photo_ai_for_job("missing-job", &result).unwrap());
    assert_eq!(db.ai_revision().unwrap(), ai_before);

    db.next_pending_tagging_job().unwrap().unwrap();
    assert!(db.save_photo_ai_for_job(&job, &result).unwrap());
    assert_eq!(db.photo_ai(photo).unwrap().unwrap().plugin_id, AI_PLUGIN);
    assert!(db.ai_revision().unwrap() > ai_before);
    assert_eq!(db.library_revision().unwrap(), library_before);
    db.complete_tagging_job(&job).unwrap();
    assert_eq!(
        db.tagging_job(&job).unwrap().unwrap().status,
        TaggingJobStatus::Completed
    );

    let missing_photo = seed_other_photo(&db);
    let missing_job = claim_tagging_job(&db, missing_photo);
    db.mark_missing(missing_photo).unwrap();
    assert!(!db.save_photo_ai_for_job(&missing_job.id, &result).unwrap());
    assert!(db.photo_ai(missing_photo).unwrap().is_none());
}

#[test]
fn resume_processing_as_pending() {
    let db = Db::open_in_memory().unwrap();
    let first = seed_photo(&db);
    let second = seed_other_photo(&db);
    db.enqueue_tagging_jobs(&[first], AI_PLUGIN, TaggingJobStatus::Pending)
        .unwrap();
    db.enqueue_tagging_jobs(&[second], AI_PLUGIN, TaggingJobStatus::Pending)
        .unwrap();
    let claimed = db.next_pending_tagging_job().unwrap().unwrap();
    assert_eq!(claimed.photo_id, first);

    assert_eq!(db.resume_interrupted_tagging_jobs().unwrap(), 1);
    let resumed = db.tagging_job(&claimed.id).unwrap().unwrap();
    assert_eq!(resumed.status, TaggingJobStatus::Pending);
    assert_eq!(resumed.retry_count, 0);
    assert_eq!(resumed.started_at, None);
    assert_eq!(db.tagging_job_counts(AI_PLUGIN).unwrap().processing, 0);
    assert_eq!(db.tagging_job_counts(AI_PLUGIN).unwrap().pending, 2);

    let reclaimed = db.next_pending_tagging_job().unwrap().unwrap();
    assert_eq!(reclaimed.id, claimed.id);
    db.complete_tagging_job(&claimed.id).unwrap();
    assert_eq!(db.resume_interrupted_tagging_jobs().unwrap(), 0);
    assert_eq!(
        db.tagging_job(&claimed.id).unwrap().unwrap().status,
        TaggingJobStatus::Completed
    );
}

#[test]
fn cancel_tagging_jobs_for_photo_only_touches_active_jobs() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let completed = seed_other_photo(&db);
    let job = claim_tagging_job(&db, completed);
    db.complete_tagging_job(&job.id).unwrap();
    db.enqueue_tagging_jobs(&[photo], AI_PLUGIN, TaggingJobStatus::Pending)
        .unwrap();
    db.enqueue_tagging_jobs(&[photo], "other-plugin", TaggingJobStatus::Waiting)
        .unwrap();

    assert_eq!(db.cancel_tagging_jobs_for_photo(photo).unwrap(), 2);
    for job in db.tagging_jobs_for_photo(photo).unwrap() {
        assert_eq!(job.status, TaggingJobStatus::Cancelled);
    }
    assert_eq!(
        db.tagging_job(&job.id).unwrap().unwrap().status,
        TaggingJobStatus::Completed
    );
}

#[test]
fn cancel_tagging_for_forced_deletions_only_touches_forced_pending_requests() {
    let db = Db::open_in_memory().unwrap();
    let forced = seed_photo(&db);
    let plain = seed_other_photo(&db);
    db.enqueue_tagging_jobs(&[forced], AI_PLUGIN, TaggingJobStatus::Pending)
        .unwrap();
    db.enqueue_tagging_jobs(&[plain], AI_PLUGIN, TaggingJobStatus::Pending)
        .unwrap();
    db.request_deletion(&[forced], DeletionReason::Manual, true)
        .unwrap();
    db.request_deletion(&[plain], DeletionReason::Manual, false)
        .unwrap();

    assert_eq!(db.cancel_tagging_for_forced_deletions().unwrap(), 1);
    assert_eq!(
        db.tagging_jobs_for_photo(forced).unwrap()[0].status,
        TaggingJobStatus::Cancelled
    );
    assert_eq!(
        db.tagging_jobs_for_photo(plain).unwrap()[0].status,
        TaggingJobStatus::Pending
    );
}

#[test]
fn active_tagging_jobs_exclude_terminal_jobs() {
    let db = Db::open_in_memory().unwrap();
    let photos = seed_photos(&db, 3);
    for photo in photos {
        db.enqueue_tagging_jobs(&[photo], AI_PLUGIN, TaggingJobStatus::Pending)
            .unwrap();
    }
    assert_eq!(db.active_tagging_jobs(10).unwrap().len(), 3);
    assert_eq!(db.active_tagging_jobs(2).unwrap().len(), 2);

    let claimed = db.next_pending_tagging_job().unwrap().unwrap();
    db.complete_tagging_job(&claimed.id).unwrap();
    assert_eq!(db.active_tagging_jobs(10).unwrap().len(), 2);
}

#[test]
fn delete_ai_data_removes_only_the_plugin_rows() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    install_test_catalog(&db);
    db.save_photo_ai(photo, AI_PLUGIN, &test_ai_result(&[("sea", 0.97)]))
        .unwrap();
    db.set_ai_tag_threshold(AI_PLUGIN, "sea", 0.8).unwrap();
    seed_ai_tag(&db, photo, "other-plugin", "sea", 0.9, true);

    db.delete_ai_data(AI_PLUGIN).unwrap();

    assert!(db.photo_ai(photo).unwrap().is_none());
    assert!(db.ai_tag_definitions(AI_PLUGIN).unwrap().is_empty());
    assert!(db.ai_tag_thresholds(AI_PLUGIN).unwrap().is_empty());
    let remaining = db.photo_ai_tags(photo, false).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].plugin_id, "other-plugin");
}

#[test]
fn delete_ai_data_cancels_active_jobs_and_blocks_late_results() {
    let db = Db::open_in_memory().unwrap();
    let processing_photo = seed_photo(&db);
    install_test_catalog(&db);
    let processing = claim_tagging_job(&db, processing_photo);
    let waiting_photo = seed_other_photo(&db);
    db.enqueue_tagging_jobs(&[waiting_photo], AI_PLUGIN, TaggingJobStatus::Waiting)
        .unwrap();
    let other_plugin_photo = seed_photos(&db, 3)[2];
    db.enqueue_tagging_jobs(
        &[other_plugin_photo],
        "other-plugin",
        TaggingJobStatus::Pending,
    )
    .unwrap();

    db.delete_ai_data(AI_PLUGIN).unwrap();

    let job = db.tagging_job(&processing.id).unwrap().unwrap();
    assert_eq!(job.status, TaggingJobStatus::Cancelled);
    assert_eq!(job.progress, 0.0);
    assert_eq!(job.stage, None);
    assert_eq!(
        db.tagging_jobs_for_photo(waiting_photo).unwrap()[0].status,
        TaggingJobStatus::Cancelled
    );
    assert_eq!(
        db.tagging_jobs_for_photo(other_plugin_photo).unwrap()[0].status,
        TaggingJobStatus::Pending
    );

    assert!(
        !db.save_photo_ai_for_job(&processing.id, &test_ai_result(&[("sea", 0.97)]))
            .unwrap()
    );
    assert!(db.photo_ai(processing_photo).unwrap().is_none());
    assert!(
        db.photo_ai_tags(processing_photo, false)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn deletion_summary_includes_tagging() {
    let db = Db::open_in_memory().unwrap();
    let photos = seed_photos(&db, 4);

    db.enqueue_tagging_jobs(&[photos[0]], AI_PLUGIN, TaggingJobStatus::Waiting)
        .unwrap();
    db.enqueue_tagging_jobs(&[photos[2]], AI_PLUGIN, TaggingJobStatus::Pending)
        .unwrap();
    let processing = db.next_pending_tagging_job().unwrap().unwrap();
    assert_eq!(processing.photo_id, photos[2]);
    db.enqueue_tagging_jobs(&[photos[3]], AI_PLUGIN, TaggingJobStatus::Pending)
        .unwrap();
    let failed = db.next_pending_tagging_job().unwrap().unwrap();
    assert_eq!(failed.photo_id, photos[3]);
    db.fail_tagging_job(&failed.id, TaggingErrorCode::Internal)
        .unwrap();
    db.enqueue_tagging_jobs(&[photos[1]], AI_PLUGIN, TaggingJobStatus::Pending)
        .unwrap();

    let summary = db.deletion_job_summary(&photos).unwrap();
    assert_eq!(summary.tagging.waiting, 1);
    assert_eq!(summary.tagging.pending, 1);
    assert_eq!(summary.tagging.processing, 1);
    assert_eq!(summary.tagging.failed, 1);
    assert_eq!(summary.affected_photos, 4);
    assert!(summary.needs_warning());
    assert_eq!(summary.compression.total(), 0);
    assert_eq!(summary.send.total(), 0);
}

#[test]
fn deletion_summary_excludes_terminal_tagging_jobs() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let job = claim_tagging_job(&db, photo);
    db.cancel_tagging_job(&job.id).unwrap();

    let summary = db.deletion_job_summary(&[photo]).unwrap();
    assert_eq!(summary.tagging.total(), 0);
    assert_eq!(summary.affected_photos, 0);
    assert!(!summary.needs_warning());
}

#[test]
fn deletion_blockers_wait_for_tagging() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    db.enqueue_tagging_jobs(&[photo], AI_PLUGIN, TaggingJobStatus::Pending)
        .unwrap();
    assert_eq!(db.deletion_blockers(photo).unwrap().active, 1);

    let request = db
        .request_deletion(&[photo], DeletionReason::Manual, false)
        .unwrap()[0];
    assert!(db.claim_deletion(request).unwrap().is_none());
    let job = db.next_pending_tagging_job().unwrap().unwrap();
    db.cancel_tagging_job(&job.id).unwrap();
    assert_eq!(db.deletion_blockers(photo).unwrap().active, 0);
    assert!(db.claim_deletion(request).unwrap().is_some());
}

#[test]
fn failed_tagging_blocks_a_non_forced_deletion() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let job = claim_tagging_job(&db, photo);
    db.fail_tagging_job(&job.id, TaggingErrorCode::Internal)
        .unwrap();
    assert_eq!(db.deletion_blockers(photo).unwrap().failed, 1);

    let request = db
        .request_deletion(&[photo], DeletionReason::Manual, false)
        .unwrap()[0];
    assert!(db.claim_deletion(request).unwrap().is_none());
    assert!(db.block_deletion_if_jobs_failed(request).unwrap());
    let blocked = db.deletion_request_by_id(request).unwrap().unwrap();
    assert_eq!(blocked.blocked_reason.as_deref(), Some("job_failed"));

    assert!(db.force_deletion(request).unwrap());
    assert!(db.claim_deletion(request).unwrap().is_some());
}

#[test]
fn force_delete_cancels_tagging_before_claiming() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    install_test_catalog(&db);
    let job = claim_tagging_job(&db, photo);
    assert_eq!(
        db.tagging_job(&job.id).unwrap().unwrap().status,
        TaggingJobStatus::Processing
    );

    let request = db
        .request_deletion(&[photo], DeletionReason::Manual, false)
        .unwrap()[0];
    assert!(db.claim_deletion(request).unwrap().is_none());
    assert!(db.force_deletion(request).unwrap());
    assert_eq!(
        db.tagging_job(&job.id).unwrap().unwrap().status,
        TaggingJobStatus::Cancelled
    );
    assert!(
        !db.save_photo_ai_for_job(&job.id, &test_ai_result(&[("sea", 0.97)]))
            .unwrap()
    );
    assert!(db.photo_ai(photo).unwrap().is_none());
    assert!(db.claim_deletion(request).unwrap().is_some());
}

#[test]
fn enqueue_missing_world_name_lookups_queues_only_unresolved_referenced_worlds() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo_a = db.upsert_photo(&sample_photo(root, "a.png", 100)).unwrap();
    let photo_b = db.upsert_photo(&sample_photo(root, "b.png", 200)).unwrap();
    let photo_c = db.upsert_photo(&sample_photo(root, "c.png", 300)).unwrap();

    let unresolved = db.upsert_world("wrld_unresolved", None, None).unwrap();
    let known = db
        .upsert_world("wrld_known", Some("Known"), Some("log"))
        .unwrap();
    let xmp_only = db.upsert_world("wrld_xmp", None, None).unwrap();
    let unreferenced = db.upsert_world("wrld_unreferenced", None, None).unwrap();
    let aliased = db.upsert_world("wrld_alias", None, None).unwrap();
    db.conn
        .execute(
            "UPDATE worlds SET alias = ?1 WHERE id = ?2",
            params!["Alias", aliased],
        )
        .unwrap();

    for (photo, world, name) in [
        (photo_a, unresolved, None),
        (photo_b, known, None),
        (photo_c, xmp_only, Some("XMP World")),
    ] {
        db.set_photo_context(&PhotoContext {
            photo_id: photo,
            session_id: None,
            world_id: Some(world),
            world_name: name.map(str::to_string),
            instance_id: None,
            link_method: "log".to_string(),
            confidence: 0.9,
            author: None,
            linked_at: "2026-11-03T00:00:00+00:00".to_string(),
        })
        .unwrap();
    }

    assert_eq!(db.enqueue_missing_world_name_lookups().unwrap(), 1);
    assert_eq!(db.enqueue_missing_world_name_lookups().unwrap(), 0);
    let queued = db.next_due_world_name_lookup(0).unwrap().unwrap();
    assert_eq!(queued.world_db_id, unresolved);
    assert_eq!(queued.world_id, "wrld_unresolved");
    assert_eq!(queued.status, WorldNameLookupStatus::Pending);
    assert!(db.next_due_world_name_lookup(0).unwrap().is_some());
    assert!(db.world_by_id(unreferenced).unwrap().is_some());
}

#[test]
fn next_due_world_name_lookup_respects_schedule_and_not_found() {
    let db = Db::open_in_memory().unwrap();
    let pending = db.upsert_world("wrld_pending", None, None).unwrap();
    let retry = db.upsert_world("wrld_retry", None, None).unwrap();
    let not_found = db.upsert_world("wrld_404", None, None).unwrap();
    db.enqueue_world_name_lookup(pending).unwrap();
    db.enqueue_world_name_lookup(retry).unwrap();
    db.enqueue_world_name_lookup(not_found).unwrap();
    db.record_world_name_failure(retry, WorldNameFailure::Retryable { code: "http_5xx" }, 100)
        .unwrap();
    db.record_world_name_failure(not_found, WorldNameFailure::NotFound, 100)
        .unwrap();

    assert_eq!(
        db.next_due_world_name_lookup(100)
            .unwrap()
            .unwrap()
            .world_db_id,
        pending
    );
    assert_eq!(
        db.next_due_world_name_lookup(159)
            .unwrap()
            .unwrap()
            .world_db_id,
        pending
    );
    db.mark_world_name_lookup_resolved(pending, 159).unwrap();
    assert_eq!(
        db.next_due_world_name_lookup(160)
            .unwrap()
            .unwrap()
            .world_db_id,
        retry
    );
    assert_eq!(
        db.next_due_world_name_lookup(86_499)
            .unwrap()
            .unwrap()
            .world_db_id,
        retry
    );
    db.mark_world_name_lookup_resolved(retry, 86_499).unwrap();
    assert_eq!(
        db.next_due_world_name_lookup(86_500)
            .unwrap()
            .unwrap()
            .world_db_id,
        not_found
    );
}

#[test]
fn failures_schedule_the_expected_backoff() {
    let db = Db::open_in_memory().unwrap();
    let world = db.upsert_world("wrld_test", None, None).unwrap();
    db.enqueue_world_name_lookup(world).unwrap();

    db.record_world_name_failure(
        world,
        WorldNameFailure::RateLimited { retry_after: 120 },
        1_000,
    )
    .unwrap();
    let row = db.world_name_lookup(world).unwrap().unwrap();
    assert_eq!(row.status, WorldNameLookupStatus::Retry);
    assert_eq!(row.attempts, 0);
    assert_eq!(row.next_attempt_at, Some(1_120));

    db.record_world_name_failure(
        world,
        WorldNameFailure::Retryable { code: "http_5xx" },
        2_000,
    )
    .unwrap();
    let row = db.world_name_lookup(world).unwrap().unwrap();
    assert_eq!(row.status, WorldNameLookupStatus::Retry);
    assert_eq!(row.attempts, 1);
    assert_eq!(row.next_attempt_at, Some(2_060));

    db.record_world_name_failure(world, WorldNameFailure::NotFound, 3_000)
        .unwrap();
    let row = db.world_name_lookup(world).unwrap().unwrap();
    assert_eq!(row.status, WorldNameLookupStatus::NotFound);
    assert_eq!(row.next_attempt_at, Some(89_400));

    db.record_world_name_failure(
        world,
        WorldNameFailure::Invalid {
            code: "invalid_response",
        },
        4_000,
    )
    .unwrap();
    let row = db.world_name_lookup(world).unwrap().unwrap();
    assert_eq!(row.status, WorldNameLookupStatus::Failed);
    assert_eq!(row.next_attempt_at, None);
}

#[test]
fn short_retry_after_never_schedules_sooner_than_the_global_interval() {
    let db = Db::open_in_memory().unwrap();
    let world = db.upsert_world("wrld_test", None, None).unwrap();
    db.enqueue_world_name_lookup(world).unwrap();

    db.record_world_name_failure(
        world,
        WorldNameFailure::RateLimited { retry_after: 30 },
        1_000,
    )
    .unwrap();
    let row = db.world_name_lookup(world).unwrap().unwrap();
    assert_eq!(row.status, WorldNameLookupStatus::Retry);
    assert_eq!(row.attempts, 0);
    assert_eq!(row.next_attempt_at, Some(1_060));
}

#[test]
fn satisfied_and_resolved_lookups_clear_the_last_error() {
    let db = Db::open_in_memory().unwrap();
    let satisfied = db.upsert_world("wrld_satisfied", None, None).unwrap();
    let resolved = db.upsert_world("wrld_resolved", None, None).unwrap();
    db.enqueue_world_name_lookup(satisfied).unwrap();
    db.enqueue_world_name_lookup(resolved).unwrap();
    db.record_world_name_failure(
        satisfied,
        WorldNameFailure::Retryable { code: "http_5xx" },
        1_000,
    )
    .unwrap();
    db.record_world_name_failure(
        resolved,
        WorldNameFailure::Retryable { code: "network" },
        1_000,
    )
    .unwrap();

    db.mark_world_name_lookup_satisfied(satisfied).unwrap();
    db.mark_world_name_lookup_resolved(resolved, 1_060).unwrap();

    for world in [satisfied, resolved] {
        let row = db.world_name_lookup(world).unwrap().unwrap();
        assert_eq!(row.status, WorldNameLookupStatus::Resolved);
        assert_eq!(row.next_attempt_at, None);
        let last_error: Option<String> = db
            .conn
            .query_row(
                "SELECT last_error FROM world_name_lookups WHERE world_db_id = ?1",
                params![world],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(last_error, None);
    }
}

#[test]
fn next_request_at_and_counts_roundtrip() {
    let db = Db::open_in_memory().unwrap();
    assert_eq!(db.world_name_next_request_at().unwrap(), None);
    db.set_world_name_next_request_at(1_700_000_000).unwrap();
    assert_eq!(
        db.world_name_next_request_at().unwrap(),
        Some(1_700_000_000)
    );

    let pending = db.upsert_world("wrld_pending", None, None).unwrap();
    let retry = db.upsert_world("wrld_retry", None, None).unwrap();
    let resolved = db.upsert_world("wrld_resolved", None, None).unwrap();
    let failed = db.upsert_world("wrld_failed", None, None).unwrap();
    for world in [pending, retry, resolved, failed] {
        db.enqueue_world_name_lookup(world).unwrap();
    }
    db.record_world_name_failure(retry, WorldNameFailure::NotFound, 0)
        .unwrap();
    db.mark_world_name_lookup_resolved(resolved, 0).unwrap();
    db.record_world_name_failure(
        failed,
        WorldNameFailure::Invalid {
            code: "invalid_response",
        },
        0,
    )
    .unwrap();

    let counts = db.world_name_lookup_counts().unwrap();
    assert_eq!(counts.pending, 2);
    assert_eq!(counts.resolved, 1);
    assert_eq!(counts.failed, 1);
}

#[test]
fn upsert_world_joins_an_existing_transaction() {
    let db = Db::open_in_memory().unwrap();
    db.in_transaction(|db| {
        let world = db.upsert_world("wrld_tx", Some("Tx World"), Some("log"))?;
        assert_eq!(
            db.world_by_id(world)?.unwrap().name.as_deref(),
            Some("Tx World")
        );
        Ok(())
    })
    .unwrap();

    let world = db.world_by_world_id("wrld_tx").unwrap().unwrap();
    assert_eq!(world.name.as_deref(), Some("Tx World"));
}

#[test]
fn backfill_photo_context_world_names_fills_nulls_with_alias_then_name() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db.upsert_photo(&sample_photo(root, "a.png", 100)).unwrap();
    let world = db
        .upsert_world("wrld_test", Some("World Name"), Some("log"))
        .unwrap();
    db.conn
        .execute(
            "UPDATE worlds SET alias = ?1 WHERE id = ?2",
            params!["Alias Name", world],
        )
        .unwrap();
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
    let before = db.library_revision().unwrap();

    assert_eq!(db.backfill_photo_context_world_names().unwrap(), 1);
    assert_eq!(
        db.photo_context(photo)
            .unwrap()
            .unwrap()
            .world_name
            .as_deref(),
        Some("Alias Name")
    );
    assert!(db.library_revision().unwrap() > before);
    assert_eq!(db.backfill_photo_context_world_names().unwrap(), 0);
}

#[test]
fn record_world_name_resolved_fills_only_nulls_and_keeps_log_names() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let null_photo = db.upsert_photo(&sample_photo(root, "a.png", 100)).unwrap();
    let xmp_photo = db.upsert_photo(&sample_photo(root, "b.png", 200)).unwrap();
    let world = db.upsert_world("wrld_test", None, None).unwrap();
    for (photo, name) in [(null_photo, None), (xmp_photo, Some("XMP World"))] {
        db.set_photo_context(&PhotoContext {
            photo_id: photo,
            session_id: None,
            world_id: Some(world),
            world_name: name.map(str::to_string),
            instance_id: None,
            link_method: "log".to_string(),
            confidence: 0.9,
            author: None,
            linked_at: "2026-11-03T00:00:00+00:00".to_string(),
        })
        .unwrap();
    }
    db.enqueue_world_name_lookup(world).unwrap();

    db.record_world_name_resolved(world, "API World", 1_000)
        .unwrap();
    let stored = db.world_by_id(world).unwrap().unwrap();
    assert_eq!(stored.name.as_deref(), Some("API World"));
    assert_eq!(stored.name_source.as_deref(), Some("vrchat-api"));
    assert_eq!(
        db.photo_context(null_photo)
            .unwrap()
            .unwrap()
            .world_name
            .as_deref(),
        Some("API World")
    );
    assert_eq!(
        db.photo_context(xmp_photo)
            .unwrap()
            .unwrap()
            .world_name
            .as_deref(),
        Some("XMP World")
    );
    assert_eq!(
        db.world_name_lookup(world).unwrap().unwrap().status,
        WorldNameLookupStatus::Resolved
    );

    db.record_world_name_resolved(world, "Ignored API World", 2_000)
        .unwrap();
    assert_eq!(
        db.world_by_id(world).unwrap().unwrap().name.as_deref(),
        Some("API World")
    );
}

#[test]
fn api_result_after_log_name_fills_null_contexts_with_the_log_name() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db.upsert_photo(&sample_photo(root, "a.png", 100)).unwrap();
    let world = db
        .upsert_world("wrld_test", Some("Log World"), Some("log"))
        .unwrap();
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
    db.enqueue_world_name_lookup(world).unwrap();

    db.record_world_name_resolved(world, "API World", 1_000)
        .unwrap();
    assert_eq!(
        db.world_by_id(world).unwrap().unwrap().name.as_deref(),
        Some("Log World")
    );
    assert_eq!(
        db.photo_context(photo)
            .unwrap()
            .unwrap()
            .world_name
            .as_deref(),
        Some("Log World")
    );
}

#[test]
fn log_name_overwrites_api_name_and_updates_contexts() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db.upsert_photo(&sample_photo(root, "a.png", 100)).unwrap();
    let world = db
        .upsert_world("wrld_test", Some("API World"), Some("vrchat-api"))
        .unwrap();
    db.set_photo_context(&PhotoContext {
        photo_id: photo,
        session_id: None,
        world_id: Some(world),
        world_name: Some("API World".to_string()),
        instance_id: None,
        link_method: "log".to_string(),
        confidence: 0.9,
        author: None,
        linked_at: "2026-11-03T00:00:00+00:00".to_string(),
    })
    .unwrap();
    db.enqueue_world_name_lookup(world).unwrap();

    db.upsert_world("wrld_test", Some("Log World"), Some("log"))
        .unwrap();
    let stored = db.world_by_id(world).unwrap().unwrap();
    assert_eq!(stored.name.as_deref(), Some("Log World"));
    assert_eq!(stored.name_source.as_deref(), Some("log"));
    assert_eq!(
        db.photo_context(photo)
            .unwrap()
            .unwrap()
            .world_name
            .as_deref(),
        Some("Log World")
    );
    assert_eq!(
        db.world_name_lookup(world).unwrap().unwrap().status,
        WorldNameLookupStatus::Resolved
    );

    db.upsert_world("wrld_test", Some("XMP World"), Some("xmp"))
        .unwrap();
    assert_eq!(
        db.world_by_id(world).unwrap().unwrap().name.as_deref(),
        Some("Log World")
    );
}

#[test]
fn enqueue_send_dedupes_active_rows_and_allows_resend_after_terminal() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let id = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    assert!(
        db.enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Auto,
            SendStatus::Pending
        )
        .unwrap()
        .is_none()
    );
    assert!(db.cancel_send(id).unwrap());
    let again = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap();
    assert!(again.is_some());
}

#[test]
fn enqueue_send_rejects_a_deleting_photo() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    db.request_deletion(&[photo], DeletionReason::Manual, false)
        .unwrap();
    assert!(
        db.enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn claim_next_send_respects_status_and_next_attempt() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let id = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.set_send_next_attempt(id, 200).unwrap();
    assert!(db.claim_next_send(100).unwrap().is_none());
    let claimed = db.claim_next_send(200).unwrap().unwrap();
    assert_eq!(claimed.id, id);
    assert_eq!(claimed.status, SendStatus::Processing);
    assert!(db.claim_next_send(300).unwrap().is_none());
}

#[test]
fn test_send_has_no_photo() {
    let db = Db::open_in_memory().unwrap();
    let id = db.enqueue_test_send().unwrap();
    let entry = db.send_entry_by_id(id).unwrap().unwrap();
    assert_eq!(entry.kind, SendKind::Test);
    assert_eq!(entry.photo_id, None);
}

#[test]
fn send_counts_and_active_lookup_follow_statuses() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let other = seed_other_photo(&db);
    assert!(db.active_send_for_photo(photo).unwrap().is_none());

    let waiting = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Auto,
            SendStatus::Waiting,
        )
        .unwrap()
        .unwrap();
    let pending = db
        .enqueue_send(
            other,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();

    assert_eq!(db.count_waiting_sends().unwrap(), 1);
    assert_eq!(db.count_pending_sends().unwrap(), 2);
    assert_eq!(
        db.active_send_for_photo(photo).unwrap().unwrap().id,
        waiting
    );

    db.claim_next_send(100).unwrap();
    assert_eq!(db.count_waiting_sends().unwrap(), 1);
    assert_eq!(db.count_pending_sends().unwrap(), 1);
    assert_eq!(
        db.active_send_for_photo(other).unwrap().unwrap().id,
        pending
    );

    db.conn
        .execute(
            "UPDATE send_outbox SET status = 'sent' WHERE id = ?1",
            params![waiting],
        )
        .unwrap();
    assert!(db.active_send_for_photo(photo).unwrap().is_none());
}

#[test]
fn send_status_parse_keeps_unknown_values_distinct() {
    assert_eq!(SendStatus::parse("future_state"), SendStatus::Unknown);
    assert_eq!(SendStatus::Unknown.as_str(), "unknown");
    assert!(!SendStatus::Unknown.is_terminal());
    assert_eq!(SendErrorCode::parse("http_413"), SendErrorCode::Http413);
    assert_eq!(SendErrorCode::parse("future_error"), SendErrorCode::Unknown);
}

#[test]
fn enqueue_send_rejects_statuses_other_than_waiting_or_pending() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    for status in [
        SendStatus::Processing,
        SendStatus::Sent,
        SendStatus::Failed,
        SendStatus::Cancelled,
        SendStatus::Unknown,
    ] {
        assert!(matches!(
            db.enqueue_send(photo, SendKind::Photo, SendOrigin::Manual, status),
            Err(CoreError::Invalid(_))
        ));
    }
    assert_eq!(db.count_pending_sends().unwrap(), 0);
}

#[test]
fn claim_next_send_claims_oldest_due_row_first() {
    let db = Db::open_in_memory().unwrap();
    let first = seed_photo(&db);
    let second = seed_other_photo(&db);
    let first_id = db
        .enqueue_send(
            first,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    let second_id = db
        .enqueue_send(
            second,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    assert_eq!(db.claim_next_send(100).unwrap().unwrap().id, first_id);
    assert_eq!(db.claim_next_send(100).unwrap().unwrap().id, second_id);
    assert!(db.claim_next_send(100).unwrap().is_none());
}

#[test]
fn complete_send_stores_message_identity_and_source() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let id = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.claim_next_send(100).unwrap();
    db.complete_send(
        id,
        &SendCompletion {
            source: Some(SendSource::Original),
            profile_id: None,
            destination: "My server".into(),
            message_id: Some("42".into()),
            channel_id: Some("7".into()),
            guild_id: Some("9".into()),
        },
    )
    .unwrap();
    let entry = db.send_entry_by_id(id).unwrap().unwrap();
    assert_eq!(entry.status, SendStatus::Sent);
    assert_eq!(entry.discord_message_id.as_deref(), Some("42"));
    assert_eq!(entry.discord_channel_id.as_deref(), Some("7"));
    assert_eq!(entry.discord_guild_id.as_deref(), Some("9"));
    assert_eq!(entry.source, Some(SendSource::Original));
    assert_eq!(entry.destination.as_deref(), Some("My server"));
    assert!(entry.finished_at.is_some());
}

#[test]
fn complete_send_leaves_unclaimed_rows_alone() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let id = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.complete_send(
        id,
        &SendCompletion {
            source: None,
            profile_id: None,
            destination: "D".into(),
            message_id: None,
            channel_id: None,
            guild_id: None,
        },
    )
    .unwrap();
    assert_eq!(
        db.send_entry_by_id(id).unwrap().unwrap().status,
        SendStatus::Pending
    );
}

#[test]
fn fail_send_stores_error_and_identity() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let id = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.fail_send(
        id,
        SendErrorCode::Http401,
        Some("unauthorized"),
        Some(SendSource::Profile),
        Some("profile-a"),
        Some("My server"),
    )
    .unwrap();
    let entry = db.send_entry_by_id(id).unwrap().unwrap();
    assert_eq!(entry.status, SendStatus::Failed);
    assert_eq!(entry.error_code.as_deref(), Some("http_401"));
    assert_eq!(entry.error_detail.as_deref(), Some("unauthorized"));
    assert_eq!(entry.source, Some(SendSource::Profile));
    assert_eq!(entry.profile_id.as_deref(), Some("profile-a"));
    assert_eq!(entry.destination.as_deref(), Some("My server"));
    assert!(entry.finished_at.is_some());
}

#[test]
fn fail_send_after_attempts_stores_the_exhausted_count() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let id = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.claim_next_send(100).unwrap();
    db.reschedule_send(id, 9, 300, SendErrorCode::Network, Some("timeout"))
        .unwrap();
    db.claim_next_send(300).unwrap();
    db.fail_send_after_attempts(
        id,
        10,
        SendErrorCode::Network,
        Some("timeout"),
        Some(SendSource::Original),
        None,
        Some("My server"),
    )
    .unwrap();
    let entry = db.send_entry_by_id(id).unwrap().unwrap();
    assert_eq!(entry.status, SendStatus::Failed);
    assert_eq!(entry.attempts, 10);
    assert_eq!(entry.next_attempt_at, None);
    assert_eq!(entry.error_code.as_deref(), Some("network"));
    assert_eq!(entry.error_detail.as_deref(), Some("timeout"));
    assert_eq!(entry.source, Some(SendSource::Original));
    assert_eq!(entry.destination.as_deref(), Some("My server"));
    assert!(entry.finished_at.is_some());
    assert!(db.claim_next_send(10_000).unwrap().is_none());
}

#[test]
fn reschedule_send_delays_and_records_the_last_error() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let id = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.claim_next_send(100).unwrap();
    db.reschedule_send(id, 2, 300, SendErrorCode::Network, Some("timeout"))
        .unwrap();
    let entry = db.send_entry_by_id(id).unwrap().unwrap();
    assert_eq!(entry.status, SendStatus::Pending);
    assert_eq!(entry.attempts, 2);
    assert_eq!(entry.next_attempt_at, Some(300));
    assert_eq!(entry.error_code.as_deref(), Some("network"));
    assert_eq!(entry.error_detail.as_deref(), Some("timeout"));
    assert_eq!(entry.started_at, None);
    assert!(db.claim_next_send(299).unwrap().is_none());
    assert_eq!(db.claim_next_send(300).unwrap().unwrap().id, id);
}

#[test]
fn release_send_returns_a_claimed_row_to_the_queue() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let id = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.claim_next_send(100).unwrap();
    db.release_send(id).unwrap();
    let entry = db.send_entry_by_id(id).unwrap().unwrap();
    assert_eq!(entry.status, SendStatus::Pending);
    assert_eq!(entry.started_at, None);
    assert_eq!(db.claim_next_send(200).unwrap().unwrap().id, id);
}

#[test]
fn retry_send_resets_attempts_and_errors_only_for_failed() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let id = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.claim_next_send(100).unwrap();
    db.reschedule_send(id, 3, 200, SendErrorCode::Network, Some("timeout"))
        .unwrap();
    db.claim_next_send(200).unwrap();
    db.fail_send(
        id,
        SendErrorCode::Http401,
        Some("unauthorized"),
        None,
        None,
        None,
    )
    .unwrap();
    assert!(db.retry_send(id).unwrap());
    let entry = db.send_entry_by_id(id).unwrap().unwrap();
    assert_eq!(entry.status, SendStatus::Pending);
    assert_eq!(entry.attempts, 0);
    assert_eq!(entry.next_attempt_at, None);
    assert_eq!(entry.error_code, None);
    assert_eq!(entry.error_detail, None);
    assert_eq!(entry.started_at, None);
    assert_eq!(entry.finished_at, None);
    assert!(!db.retry_send(id).unwrap());
}

#[test]
fn cancel_send_refuses_processing_rows() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let id = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.claim_next_send(100).unwrap();
    assert!(!db.cancel_send(id).unwrap());
    assert_eq!(
        db.send_entry_by_id(id).unwrap().unwrap().status,
        SendStatus::Processing
    );
}

#[test]
fn cancel_send_cancels_waiting_pending_and_failed_rows() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let waiting = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Auto,
            SendStatus::Waiting,
        )
        .unwrap()
        .unwrap();
    let pending = db.enqueue_test_send().unwrap();
    let failed = db.enqueue_test_send().unwrap();
    db.fail_send(failed, SendErrorCode::Unknown, None, None, None, None)
        .unwrap();

    for id in [waiting, pending, failed] {
        assert!(db.cancel_send(id).unwrap());
        let entry = db.send_entry_by_id(id).unwrap().unwrap();
        assert_eq!(entry.status, SendStatus::Cancelled);
        assert!(entry.finished_at.is_some());
    }
    assert!(!db.cancel_send(waiting).unwrap());
}

#[test]
fn retry_send_refuses_when_another_active_row_exists() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let failed = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.claim_next_send(100).unwrap();
    db.fail_send(failed, SendErrorCode::Http401, None, None, None, None)
        .unwrap();
    let active = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    assert!(!db.retry_send(failed).unwrap());
    assert_eq!(
        db.send_entry_by_id(failed).unwrap().unwrap().status,
        SendStatus::Failed
    );
    assert_eq!(
        db.send_entry_by_id(active).unwrap().unwrap().status,
        SendStatus::Pending
    );
}

#[test]
fn reset_processing_sends_marks_interrupted_without_resending() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let id = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Manual,
            SendStatus::Pending,
        )
        .unwrap()
        .unwrap();
    db.claim_next_send(100).unwrap();
    assert_eq!(db.reset_processing_sends().unwrap(), 1);
    let entry = db.send_entry_by_id(id).unwrap().unwrap();
    assert_eq!(entry.status, SendStatus::Failed);
    assert_eq!(entry.error_code.as_deref(), Some("interrupted"));
    assert_eq!(entry.error_detail, None);
    assert!(entry.finished_at.is_some());
    assert!(db.claim_next_send(200).unwrap().is_none());
}

#[test]
fn release_waiting_and_cancel_waiting_auto_sends() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let waiting = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Auto,
            SendStatus::Waiting,
        )
        .unwrap()
        .unwrap();
    assert_eq!(db.release_waiting_sends().unwrap(), 1);
    assert_eq!(
        db.send_entry_by_id(waiting).unwrap().unwrap().status,
        SendStatus::Pending
    );
    let photo2 = seed_other_photo(&db);
    db.enqueue_send(
        photo2,
        SendKind::Photo,
        SendOrigin::Auto,
        SendStatus::Waiting,
    )
    .unwrap();
    assert_eq!(db.cancel_waiting_auto_sends().unwrap(), 1);
    assert_eq!(
        db.send_entry_by_id(waiting).unwrap().unwrap().status,
        SendStatus::Pending
    );
}

#[test]
fn send_history_joins_photo_and_world_and_prunes_to_cap() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    db.set_photo_context(&PhotoContext {
        photo_id: photo,
        session_id: None,
        world_id: None,
        world_name: Some("Test World".to_string()),
        instance_id: None,
        link_method: "manual".to_string(),
        confidence: 1.0,
        author: None,
        linked_at: now_string(),
    })
    .unwrap();
    for _ in 0..3 {
        let id = db
            .enqueue_send(
                photo,
                SendKind::Photo,
                SendOrigin::Manual,
                SendStatus::Pending,
            )
            .unwrap()
            .unwrap();
        db.claim_next_send(100).unwrap();
        db.complete_send(
            id,
            &SendCompletion {
                source: Some(SendSource::Original),
                profile_id: None,
                destination: "D".into(),
                message_id: Some(id.to_string()),
                channel_id: None,
                guild_id: None,
            },
        )
        .unwrap();
    }
    let history = db.send_history(10).unwrap();
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].entry.discord_message_id.as_deref(), Some("3"));
    assert_eq!(history[0].photo_name.as_deref(), Some("a.png"));
    assert_eq!(history[0].world_name.as_deref(), Some("Test World"));
    assert_eq!(db.send_history(1).unwrap().len(), 1);
    assert_eq!(db.prune_send_history(2).unwrap(), 1);
    assert_eq!(db.send_history(10).unwrap().len(), 2);
    assert_eq!(db.clear_send_history().unwrap(), 2);
    assert!(db.send_history(10).unwrap().is_empty());
}

#[test]
fn prune_and_clear_send_history_keep_active_rows() {
    let db = Db::open_in_memory().unwrap();
    let photo = seed_photo(&db);
    let waiting = db
        .enqueue_send(
            photo,
            SendKind::Photo,
            SendOrigin::Auto,
            SendStatus::Waiting,
        )
        .unwrap()
        .unwrap();
    for _ in 0..3 {
        let id = db.enqueue_test_send().unwrap();
        db.claim_next_send(100).unwrap();
        db.complete_send(
            id,
            &SendCompletion {
                source: None,
                profile_id: None,
                destination: "D".into(),
                message_id: None,
                channel_id: None,
                guild_id: None,
            },
        )
        .unwrap();
    }
    assert_eq!(db.prune_send_history(1).unwrap(), 2);
    assert_eq!(db.send_history(10).unwrap().len(), 2);
    assert_eq!(
        db.send_entry_by_id(waiting).unwrap().unwrap().status,
        SendStatus::Waiting
    );
    assert_eq!(db.clear_send_history().unwrap(), 1);
    assert!(db.send_entry_by_id(waiting).unwrap().is_some());
    assert_eq!(db.send_history(10).unwrap().len(), 1);
}

#[test]
fn daemon_commands_deduplicate_and_drain_in_order() {
    let db = Db::open_in_memory().unwrap();
    assert!(db.request_daemon_command(DaemonCommand::Rescan).unwrap());
    assert!(!db.request_daemon_command(DaemonCommand::Rescan).unwrap());
    assert!(db.request_daemon_command(DaemonCommand::Relog).unwrap());

    assert_eq!(
        db.take_daemon_commands().unwrap(),
        vec![DaemonCommand::Rescan, DaemonCommand::Relog]
    );
    assert!(db.take_daemon_commands().unwrap().is_empty());
    assert!(db.request_daemon_command(DaemonCommand::Relink).unwrap());
}

#[test]
fn daemon_scan_root_commands_deduplicate_per_path() {
    let db = Db::open_in_memory().unwrap();
    assert!(db.request_daemon_scan_root("/photos/imported").unwrap());
    assert!(!db.request_daemon_scan_root("/photos/imported").unwrap());
    assert!(db.request_daemon_scan_root("/photos/other").unwrap());

    assert_eq!(
        db.take_daemon_commands().unwrap(),
        vec![
            DaemonCommand::ScanRoot {
                path: "/photos/imported".into()
            },
            DaemonCommand::ScanRoot {
                path: "/photos/other".into()
            },
        ]
    );
    assert!(db.take_daemon_commands().unwrap().is_empty());
}

#[test]
fn unknown_daemon_rows_are_ignored() {
    let db = Db::open_in_memory().unwrap();
    db.connection()
        .execute(
            "INSERT INTO daemon_errors(operation, message, detail, occurred_at, attempts) \
                 VALUES ('future-op', 'boom', NULL, '2026-10-21T00:00:00+00:00', 1)",
            [],
        )
        .unwrap();
    db.record_daemon_error(DaemonOperation::Log, "log failed", Some("detail"))
        .unwrap();

    let errors = db.daemon_errors().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].operation, DaemonOperation::Log);
    assert_eq!(errors[0].detail.as_deref(), Some("detail"));
    assert_eq!(errors[0].attempts, 1);

    db.connection()
        .execute(
            "INSERT INTO daemon_commands(command, created_at) VALUES ('future-command', 'now')",
            [],
        )
        .unwrap();
    assert!(db.request_daemon_command(DaemonCommand::Rescan).unwrap());
    assert_eq!(
        db.take_daemon_commands().unwrap(),
        vec![DaemonCommand::Rescan]
    );
    assert!(db.take_daemon_commands().unwrap().is_empty());
}

#[test]
fn ensure_default_profiles_is_idempotent() {
    let db = Db::open_in_memory().unwrap();
    let first = db.ensure_default_profiles().unwrap();
    assert_eq!(first.len(), 3);

    let order: Vec<&str> = first.iter().map(|profile| profile.name.as_str()).collect();
    assert_eq!(order, ["無劣化webp", "discord用webp", "高画質webp"]);

    let by_name = |name: &str| first.iter().find(|profile| profile.name == name).unwrap();
    assert!(by_name("無劣化webp").lossless);
    assert_eq!(by_name("discord用webp").width, 2560);
    assert_eq!(by_name("discord用webp").height, 1440);
    assert_eq!(
        by_name("discord用webp").output_dir.as_deref(),
        Some("discord")
    );
    assert_eq!(by_name("discord用webp").suffix, "_discord");
    assert_eq!(by_name("高画質webp").quality, 90);
    assert_eq!(by_name("高画質webp").output_dir.as_deref(), Some("webp"));

    let first_ids: Vec<String> = first.iter().map(|profile| profile.id.clone()).collect();
    let second = db.ensure_default_profiles().unwrap();
    assert_eq!(second.len(), 3);
    let second_ids: Vec<String> = second.iter().map(|profile| profile.id.clone()).collect();
    assert_eq!(first_ids, second_ids);
    assert_eq!(db.list_profiles().unwrap().len(), 3);
}

#[test]
fn profile_json_ignores_removed_role_flags() {
    let legacy = r#"{
        "id":"p","name":"Legacy","enabled":true,
        "auto_enabled":true,"clipboard":true,"discord":true,
        "format":"webp","quality":82,"lossless":true,
        "resize_enabled":false,"resize_mode":"downscale_only",
        "width":3840,"height":2160,"maintain_aspect":true,
        "output_dir":null,"suffix":"","encode_speed":6
    }"#;
    let profile: CompressionProfile = serde_json::from_str(legacy).unwrap();
    assert_eq!(profile.name, "Legacy");
}

#[test]
fn ensure_default_profiles_keeps_existing_rows() {
    let db = Db::open_in_memory().unwrap();
    let custom = CompressionProfile {
        name: "custom".to_string(),
        ..CompressionProfile::default()
    };
    db.save_profile(&custom).unwrap();

    let profiles = db.ensure_default_profiles().unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].id, custom.id);
    assert_eq!(profiles[0].name, "custom");
}

#[test]
fn ensure_default_profiles_rolls_back_partial_seed() {
    let db = Db::open_in_memory().unwrap();
    db.connection()
        .execute_batch(
            "CREATE TRIGGER fail_second_profile BEFORE INSERT ON compression_profiles \
                 WHEN (SELECT COUNT(*) FROM compression_profiles) = 1 \
                 BEGIN SELECT RAISE(ABORT, 'simulated write failure'); END;",
        )
        .unwrap();

    assert!(db.ensure_default_profiles().is_err());
    assert_eq!(db.list_profiles().unwrap().len(), 0);
}

#[test]
fn ensure_default_profiles_does_not_seed_on_read_error() {
    let db = Db::open_in_memory().unwrap();
    db.connection()
        .execute(
            "INSERT INTO compression_profiles(id, name, enabled, json, sort_order) \
                 VALUES ('broken', 'broken', 1, '{', 0)",
            [],
        )
        .unwrap();

    assert!(db.ensure_default_profiles().is_err());
    let count: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM compression_profiles", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn save_profile_assigns_and_preserves_sort_order() {
    let db = Db::open_in_memory().unwrap();
    for (id, name) in [("c", "C"), ("a", "A"), ("b", "B")] {
        db.save_profile(&profile(id, name)).unwrap();
    }

    let ids = || {
        db.list_profiles()
            .unwrap()
            .into_iter()
            .map(|profile| profile.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(), vec!["c", "a", "b"]);

    let mut edited = profile("a", "A renamed");
    edited.quality = 44;
    db.save_profile(&edited).unwrap();

    assert_eq!(ids(), vec!["c", "a", "b"]);
    let stored = db.list_profiles().unwrap();
    assert_eq!(stored[1].name, "A renamed");
    assert_eq!(stored[1].quality, 44);

    // Deleting the middle row leaves a gap. The next insert must use
    // MAX(sort_order) + 1, not COUNT(*), so "A0" goes after "B".
    db.delete_profile("a").unwrap();
    db.save_profile(&profile("d", "A0")).unwrap();
    assert_eq!(ids(), vec!["c", "b", "d"]);
}

#[test]
fn save_profile_renames_the_name_column_and_json_together() {
    let db = Db::open_in_memory().unwrap();
    let mut saved = profile("p", "Old name");
    db.save_profile(&saved).unwrap();

    saved.name = "Archive".to_string();
    db.save_profile(&saved).unwrap();

    let (column, json): (String, String) = db
        .connection()
        .query_row(
            "SELECT name, json FROM compression_profiles WHERE id = 'p'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(column, "Archive");
    let stored: CompressionProfile = serde_json::from_str(&json).unwrap();
    assert_eq!(stored.name, "Archive");
    assert_eq!(db.list_profiles().unwrap()[0].name, "Archive");
}

#[test]
fn renamed_default_profile_survives_reopen_and_reseeding() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    {
        let db = Db::open(&path).unwrap();
        let mut renamed = db.ensure_default_profiles().unwrap()[0].clone();
        renamed.name = "アーカイブ用".to_string();
        db.save_profile(&renamed).unwrap();
    }

    let db = Db::open(&path).unwrap();
    let profiles = db.ensure_default_profiles().unwrap();
    assert_eq!(profiles[0].name, "アーカイブ用");
    assert_eq!(profiles.len(), 3);
}

#[test]
fn profiles_may_share_a_name() {
    let db = Db::open_in_memory().unwrap();
    db.save_profile(&profile("a", "同じ名前")).unwrap();
    db.save_profile(&profile("b", "同じ名前")).unwrap();

    let stored = db.list_profiles().unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].name, stored[1].name);
}

#[test]
fn reorder_profiles_persists_and_rolls_back_bad_lists() {
    let db = Db::open_in_memory().unwrap();
    for (id, name) in [("c", "C"), ("a", "A"), ("b", "B")] {
        db.save_profile(&profile(id, name)).unwrap();
    }

    let ids = || {
        db.list_profiles()
            .unwrap()
            .into_iter()
            .map(|profile| profile.id)
            .collect::<Vec<_>>()
    };
    let order = |values: &[&str]| values.iter().map(|id| id.to_string()).collect::<Vec<_>>();

    // A legacy database where every row is zero sorts by name.
    db.connection()
        .execute("UPDATE compression_profiles SET sort_order = 0", [])
        .unwrap();
    assert_eq!(ids(), vec!["a", "b", "c"]);

    db.reorder_profiles(&order(&["c", "a", "b"])).unwrap();
    assert_eq!(ids(), vec!["c", "a", "b"]);

    assert!(matches!(
        db.reorder_profiles(&order(&["c", "a"])),
        Err(CoreError::Invalid(_))
    ));
    assert_eq!(ids(), vec!["c", "a", "b"]);

    assert!(matches!(
        db.reorder_profiles(&order(&["a", "a", "b"])),
        Err(CoreError::Invalid(_))
    ));
    assert_eq!(ids(), vec!["c", "a", "b"]);

    assert!(matches!(
        db.reorder_profiles(&order(&["c", "a", "missing"])),
        Err(CoreError::NotFound(_))
    ));
    assert_eq!(ids(), vec!["c", "a", "b"]);
}

#[test]
fn reorder_profiles_survives_reopen_with_legacy_zero_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");

    {
        let db = Db::open(&path).unwrap();
        for (id, name) in [("c", "C"), ("a", "A"), ("b", "B")] {
            db.save_profile(&profile(id, name)).unwrap();
        }
        db.connection()
            .execute("UPDATE compression_profiles SET sort_order = 0", [])
            .unwrap();
        let ids: Vec<String> = db
            .list_profiles()
            .unwrap()
            .into_iter()
            .map(|profile| profile.id)
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"]);

        db.reorder_profiles(&["c".to_string(), "a".to_string(), "b".to_string()])
            .unwrap();
    }

    let db = Db::open(&path).unwrap();
    let ids: Vec<String> = db
        .list_profiles()
        .unwrap()
        .into_iter()
        .map(|profile| profile.id)
        .collect();
    assert_eq!(ids, vec!["c", "a", "b"]);
}

#[test]
fn duplicate_profile_inserts_named_copy_after_source() {
    let db = Db::open_in_memory().unwrap();
    for (id, name) in [("a", "A"), ("b", "B"), ("c", "C")] {
        db.save_profile(&profile(id, name)).unwrap();
    }
    let source = db
        .list_profiles()
        .unwrap()
        .into_iter()
        .find(|profile| profile.id == "b")
        .unwrap();

    let copy = db.duplicate_profile("b").unwrap();
    assert_ne!(copy.id, source.id);
    assert_eq!(copy.name, "B のコピー");
    assert_eq!(copy.format, source.format);
    assert_eq!(copy.quality, source.quality);

    let stored = db.list_profiles().unwrap();
    let ids: Vec<&str> = stored.iter().map(|profile| profile.id.as_str()).collect();
    assert_eq!(ids, vec!["a", "b", copy.id.as_str(), "c"]);

    let mut edited = copy.clone();
    edited.quality = 31;
    db.save_profile(&edited).unwrap();
    let stored = db.list_profiles().unwrap();
    assert_eq!(stored[1].quality, source.quality);
    assert_eq!(stored[2].quality, 31);

    let second = db.duplicate_profile("b").unwrap();
    assert_eq!(second.name, "B のコピー 2");
    let stored = db.list_profiles().unwrap();
    let ids: Vec<&str> = stored.iter().map(|profile| profile.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["a", "b", second.id.as_str(), copy.id.as_str(), "c"]
    );

    assert!(matches!(
        db.duplicate_profile("missing"),
        Err(CoreError::NotFound(_))
    ));
}

#[test]
fn duplicate_profile_caps_generated_names_at_the_ui_limit() {
    let db = Db::open_in_memory().unwrap();
    let source_name = "あ".repeat(MAX_PROFILE_NAME_CHARS);
    db.save_profile(&profile("long", &source_name)).unwrap();

    let first = db.duplicate_profile("long").unwrap();
    assert_eq!(first.name.chars().count(), MAX_PROFILE_NAME_CHARS);
    assert!(first.name.ends_with(" のコピー"));

    let second = db.duplicate_profile("long").unwrap();
    assert_eq!(second.name.chars().count(), MAX_PROFILE_NAME_CHARS);
    assert!(second.name.ends_with(" のコピー 2"));
}







#[test]
fn delete_setting_removes_only_the_requested_key() {
    let db = Db::open_in_memory().unwrap();
    db.set_setting("keep", "value").unwrap();
    db.set_setting("remove", "value").unwrap();
    db.delete_setting("remove").unwrap();
    assert_eq!(db.get_setting("remove").unwrap(), None);
    assert_eq!(db.get_setting("keep").unwrap().as_deref(), Some("value"));
    db.delete_setting("missing").unwrap();
}

#[test]
fn duplicate_profile_normalizes_legacy_zero_order() {
    let db = Db::open_in_memory().unwrap();
    for (id, name) in [("a", "A"), ("b", "B"), ("c", "C")] {
        db.save_profile(&profile(id, name)).unwrap();
    }
    db.connection()
        .execute("UPDATE compression_profiles SET sort_order = 0", [])
        .unwrap();

    let copy = db.duplicate_profile("b").unwrap();
    let stored = db.list_profiles().unwrap();
    let ids: Vec<&str> = stored.iter().map(|profile| profile.id.as_str()).collect();
    assert_eq!(ids, vec!["a", "b", copy.id.as_str(), "c"]);
}

#[test]
fn delete_profile_blocks_active_jobs_and_keeps_history() {
    let db = Db::open_in_memory().unwrap();
    db.save_profile(&profile("p", "P")).unwrap();
    db.save_profile(&profile("q", "Q")).unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let job = db.enqueue_job(photo, "p", "keep").unwrap();

    assert_eq!(db.list_profiles().unwrap().len(), 2);
    assert_eq!(db.active_job_count_for_profile("p").unwrap(), 1);
    assert!(matches!(db.delete_profile("p"), Err(CoreError::Invalid(_))));
    assert_eq!(db.list_profiles().unwrap().len(), 2);

    db.update_job(&job, QueueStatus::Processing, 0.1, None, None)
        .unwrap();
    assert_eq!(db.active_job_count_for_profile("p").unwrap(), 1);
    assert!(db.delete_profile("p").is_err());

    db.update_job(
        &job,
        QueueStatus::Completed,
        1.0,
        None,
        Some("/photos/compressed/a.webp"),
    )
    .unwrap();
    db.add_derivative(
        photo,
        "/photos/compressed/a.webp",
        "webp",
        16,
        10,
        10,
        Some("p"),
        CompressionOrigin::Manual,
        "keep",
    )
    .unwrap();
    let failed = db.enqueue_job(photo, "p", "keep").unwrap();
    db.update_job(&failed, QueueStatus::Failed, 0.0, Some("boom"), None)
        .unwrap();
    assert_eq!(db.active_job_count_for_profile("p").unwrap(), 0);

    db.delete_profile("p").unwrap();
    assert_eq!(db.list_profiles().unwrap().len(), 1);
    assert_eq!(db.list_profiles().unwrap()[0].id, "q");
    assert_eq!(db.list_jobs(10).unwrap().len(), 2);
    let derivatives = db.derivatives_for_photo(photo).unwrap();
    assert_eq!(derivatives.len(), 1);
    assert_eq!(derivatives[0].profile_id.as_deref(), Some("p"));

    assert!(matches!(db.delete_profile("q"), Err(CoreError::Invalid(_))));
    assert_eq!(db.list_profiles().unwrap().len(), 1);
    assert!(matches!(
        db.delete_profile("missing"),
        Err(CoreError::NotFound(_))
    ));
}

#[test]
fn guarded_enqueue_skips_missing_profiles() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();

    assert!(
        !db.enqueue_job_for_existing_profile(photo, "missing", "keep")
            .unwrap()
    );
    assert_eq!(db.count_jobs(QueueStatus::Pending).unwrap(), 0);

    db.save_profile(&profile("p", "P")).unwrap();
    assert!(
        db.enqueue_job_for_existing_profile(photo, "p", "keep")
            .unwrap()
    );
    assert_eq!(db.count_jobs(QueueStatus::Pending).unwrap(), 1);
}

#[test]
fn add_derivative_rejects_a_path_owned_by_another_photo() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let first = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let second = db
        .upsert_photo(&sample_photo(root, "/photos/b.png", 200))
        .unwrap();
    let path = "/photos/compressed/shared.webp";

    let id = db
        .add_derivative(first, path, "webp", 10, 20, 30, Some("p"), CompressionOrigin::Manual, "keep")
        .unwrap();
    assert!(matches!(
        db.add_derivative(second, path, "webp", 10, 20, 30, Some("p"), CompressionOrigin::Manual, "keep"),
        Err(CoreError::Invalid(_))
    ));
    let first_rows = db.derivatives_for_photo(first).unwrap();
    assert_eq!(first_rows.len(), 1);
    assert_eq!(first_rows[0].id, id);
    assert!(db.derivatives_for_photo(second).unwrap().is_empty());
}

#[test]
fn replaced_photo_stays_visible_and_stats_use_derivative_size() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let id = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let derivative_id = db
        .add_derivative(
            id,
            "/photos/compressed/a.webp",
            "webp",
            16,
            10,
            1_234,
            Some("p"),
            CompressionOrigin::Manual,
            "replace",
        )
        .unwrap();

    assert!(db.set_replaced(id, derivative_id).unwrap());

    let photo = db.photo_by_id(id).unwrap().unwrap();
    assert_eq!(photo.status, PhotoStatus::Replaced);
    assert_eq!(
        photo.display_path.as_deref(),
        Some("/photos/compressed/a.webp")
    );
    assert_eq!(photo.effective_path(), "/photos/compressed/a.webp");
    assert_eq!(photo.effective_file_size(), 1_234);
    assert_eq!(photo.effective_dimensions(), (16, 10));

    assert_eq!(db.count_photos().unwrap(), 1);
    assert_eq!(db.photo_index(PhotoSort::Newest).unwrap().len(), 1);
    assert_eq!(
        db.photo_index_text(&PhotoTextFilter::default())
            .unwrap()
            .len(),
        1
    );

    let stats = db.stats().unwrap();
    assert_eq!(stats.photo_count, 1);
    assert_eq!(stats.total_bytes, 1_234);
    assert_eq!(stats.derivative_bytes, 1_234);
}

#[test]
fn upserting_a_restored_original_clears_the_display_derivative() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let id = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let derivative_id = db
        .add_derivative(
            id,
            "/photos/compressed/a.webp",
            "webp",
            16,
            10,
            1_234,
            Some("p"),
            CompressionOrigin::Manual,
            "replace",
        )
        .unwrap();
    assert!(db.set_replaced(id, derivative_id).unwrap());

    db.upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();

    let photo = db.photo_by_id(id).unwrap().unwrap();
    assert_eq!(photo.status, PhotoStatus::Present);
    assert!(photo.display_derivative_id.is_none());
    assert!(photo.display_path.is_none());
    assert_eq!(photo.effective_path(), "/photos/a.png");
    assert_eq!(db.stats().unwrap().total_bytes, 12_400_000);
}

#[test]
fn upsert_photos_returns_ids_in_input_order_and_commits() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photos = vec![
        photo_at(root, "/photos/a.png", 100, "2025-05-01 08:00:00"),
        photo_at(root, "/photos/b.png", 200, "2025-05-01 08:00:01"),
        photo_at(root, "/photos/c.png", 300, "2025-05-01 08:00:02"),
    ];

    let ids = db.upsert_photos(&photos).unwrap();

    assert_eq!(ids.len(), 3);
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(db.count_photos().unwrap(), 3);
    for (id, photo) in ids.iter().zip(photos.iter()) {
        assert_eq!(
            db.photo_by_id(*id).unwrap().unwrap().abs_path,
            photo.abs_path
        );
    }
}

#[test]
fn upsert_preserves_a_photo_status_written_by_a_newer_version() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    db.connection()
        .execute(
            "UPDATE photos SET status = 'future_status' WHERE id = ?1",
            params![photo],
        )
        .unwrap();

    db.upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();

    let status: String = db
        .connection()
        .query_row(
            "SELECT status FROM photos WHERE id = ?1",
            params![photo],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        status, "future_status",
        "an older scan must not rewrite a future status to present"
    );
    assert_eq!(
        db.photo_by_id(photo).unwrap().unwrap().status,
        PhotoStatus::Unknown
    );
}

#[test]
fn library_revision_starts_at_zero_and_is_monotonic() {
    let db = Db::open_in_memory().unwrap();
    assert_eq!(db.library_revision().unwrap(), 0);
    assert_eq!(db.bump_library_revision().unwrap(), 1);
    assert_eq!(db.bump_library_revision().unwrap(), 2);
    assert_eq!(db.library_revision().unwrap(), 2);
}

#[test]
fn rebinding_a_photo_preserves_its_id_and_user_metadata() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let id = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    db.set_favorite(id, true).unwrap();
    db.set_note(id, Some("keep me")).unwrap();
    db.set_photo_tags(id, &["night".to_string()]).unwrap();

    let mut moved = sample_photo(root, "/photos/2026/a.png", 999);
    moved.captured_at_local = "2030-01-01 00:00:00".to_string();
    moved.width = 1;
    moved.height = 1;
    moved.blake3 = Some("hash-a".to_string());
    assert!(db.rebind_photo(id, &moved).unwrap());

    let photo = db.photo_by_id(id).unwrap().unwrap();
    assert_eq!(photo.abs_path, "/photos/2026/a.png");
    assert_eq!(photo.rel_path, "photos/2026/a.png");
    assert_eq!(photo.status, PhotoStatus::Present);
    assert_eq!(photo.captured_at_utc, 100);
    assert_eq!(photo.captured_at_local, "2026-10-21 21:14:03");
    assert_eq!((photo.width, photo.height), (3840, 2160));
    assert!(photo.is_favorite);
    assert_eq!(photo.note.as_deref(), Some("keep me"));
    assert_eq!(photo.blake3.as_deref(), Some("hash-a"));
    assert_eq!(db.photo_tags(id).unwrap(), vec!["night".to_string()]);
    assert_eq!(db.count_photos().unwrap(), 1);
}

#[test]
fn rebind_candidate_prefers_a_unique_content_hash() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let id = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let mut moved = sample_photo(root, "/photos/2026/a.png", 100);
    moved.blake3 = Some("same-content".to_string());
    db.connection()
        .execute(
            "UPDATE photos SET blake3 = 'same-content' WHERE id = ?1",
            params![id],
        )
        .unwrap();

    let candidate = db.find_rebind_candidate(&moved).unwrap().unwrap();
    assert_eq!(candidate.id, id);

    moved.blake3 = Some("different-content".to_string());
    assert!(db.find_rebind_candidate(&moved).unwrap().is_none());
}

#[test]
fn duplicate_hashes_are_reported_without_merging_rows() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let mut first = sample_photo(root, "/photos/a.png", 100);
    first.blake3 = Some("same-content".to_string());
    let first_id = db.upsert_photo(&first).unwrap();
    let mut copy = sample_photo(root, "/photos/b.png", 101);
    copy.blake3 = Some("same-content".to_string());
    let copy_id = db.upsert_photo(&copy).unwrap();
    let mut other = sample_photo(root, "/photos/c.png", 102);
    other.blake3 = Some("other-content".to_string());
    db.upsert_photo(&other).unwrap();

    assert_eq!(db.duplicate_photo_ids().unwrap(), vec![first_id, copy_id]);
    assert_eq!(db.count_photos().unwrap(), 3);
}

#[test]
fn photo_query_includes_the_vrchat_world_id() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db.upsert_photo(&sample_photo(root, "a.png", 100)).unwrap();
    let world = db
        .upsert_world("wrld_test", Some("Test World"), Some("log"))
        .unwrap();
    db.set_photo_context(&PhotoContext {
        photo_id: photo,
        session_id: None,
        world_id: Some(world),
        world_name: Some("Test World".to_string()),
        instance_id: None,
        link_method: "log".to_string(),
        confidence: 0.9,
        author: None,
        linked_at: "2026-11-03T00:00:00+00:00".to_string(),
    })
    .unwrap();

    let stored = db.photo_by_id(photo).unwrap().unwrap();
    assert_eq!(stored.world_id.as_deref(), Some("wrld_test"));
}

#[test]
fn clamp_limit_uses_one_documented_policy() {
    assert_eq!(clamp_limit(i64::MIN), 1);
    assert_eq!(clamp_limit(-1), 1);
    assert_eq!(clamp_limit(0), 1);
    assert_eq!(clamp_limit(1), 1);
    assert_eq!(clamp_limit(42), 42);
    assert_eq!(clamp_limit(MAX_ROW_LIMIT), MAX_ROW_LIMIT);
    assert_eq!(clamp_limit(MAX_ROW_LIMIT + 1), MAX_ROW_LIMIT);
    assert_eq!(clamp_limit(i64::MAX), MAX_ROW_LIMIT);
}

#[test]
fn photo_list_and_search_limits_agree() {
    let db = Db::open_in_memory().unwrap();
    seed_photos(&db, 3);
    assert_limit_contract("list_photos", |limit| {
        db.list_photos(limit, 0, PhotoSort::Newest).unwrap().len()
    });
    assert_limit_contract("photo_index_text_limited", |limit| {
        db.photo_index_text_limited(&PhotoTextFilter::default(), limit)
            .unwrap()
            .len()
    });
    assert_limit_contract("search_photos", |limit| {
        db.search_photos(&PhotoFilter {
            limit,
            ..Default::default()
        })
        .unwrap()
        .len()
    });
    assert_limit_contract("random_photos_text", |limit| {
        db.random_photos_text(&PhotoTextFilter::default(), limit, 7)
            .unwrap()
            .photos
            .len()
    });

    // The legacy filter search and the production index search must agree
    // for every normalized limit, including the old zero default.
    for limit in [0, -3, MAX_ROW_LIMIT + 1, i64::MAX, 2] {
        let indexed = db
            .photo_index_text_limited(&PhotoTextFilter::default(), limit)
            .unwrap()
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        let searched = db
            .search_photos(&PhotoFilter {
                limit,
                ..Default::default()
            })
            .unwrap()
            .into_iter()
            .map(|photo| photo.id)
            .collect::<Vec<_>>();
        assert_eq!(indexed, searched, "limit={limit}");
    }
}

#[test]
fn unlinked_hash_and_derivative_limits_use_the_shared_policy() {
    let db = Db::open_in_memory().unwrap();
    let photos = seed_photos(&db, 3);
    assert_limit_contract("unlinked_photos", |limit| {
        db.unlinked_photos(limit).unwrap().len()
    });
    assert_limit_contract("unlinked_photos_due", |limit| {
        db.unlinked_photos_due(limit, 1_000).unwrap().len()
    });
    assert_limit_contract("photos_without_hash", |limit| {
        db.photos_without_hash(limit).unwrap().len()
    });

    for (index, photo) in photos.into_iter().enumerate() {
        let derivative = db
            .add_derivative(
                photo,
                &format!("/photos/derivative-{index}.webp"),
                "webp",
                1280,
                800,
                1_000,
                Some("profile"),
                CompressionOrigin::Manual,
                "keep",
            )
            .unwrap();
        assert!(db.set_replaced(photo, derivative).unwrap());
    }
    assert_limit_contract("display_derivative_paths", |limit| {
        db.display_derivative_paths(limit).unwrap().len()
    });
}

#[test]
fn deletion_request_limits_use_the_shared_policy() {
    let db = Db::open_in_memory().unwrap();
    for photo in seed_photos(&db, 3) {
        insert_deletion_request(&db, photo, "manual", "pending");
    }
    assert_limit_contract("active_deletion_requests", |limit| {
        db.active_deletion_requests(limit).unwrap().len()
    });
    assert_limit_contract("pending_deletion_requests", |limit| {
        db.pending_deletion_requests(limit).unwrap().len()
    });
}

#[test]
fn compression_job_limits_use_the_shared_policy() {
    let db = Db::open_in_memory().unwrap();
    for photo in seed_photos(&db, 3) {
        db.enqueue_job(photo, "profile", "keep").unwrap();
    }
    assert_limit_contract("list_jobs", |limit| db.list_jobs(limit).unwrap().len());
    assert_limit_contract("list_jobs_by_status", |limit| {
        db.list_jobs_by_status(QueueStatus::Pending, limit)
            .unwrap()
            .len()
    });
}

#[test]
fn send_history_and_tagging_limits_use_the_shared_policy() {
    let db = Db::open_in_memory().unwrap();
    for _ in 0..3 {
        db.enqueue_test_send().unwrap();
    }
    assert_limit_contract("send_history", |limit| {
        db.send_history(limit).unwrap().len()
    });

    let db = Db::open_in_memory().unwrap();
    for photo in seed_photos(&db, 3) {
        assert_eq!(
            db.enqueue_tagging_jobs(&[photo], AI_PLUGIN, TaggingJobStatus::Pending)
                .unwrap(),
            1
        );
    }
    assert_limit_contract("active_tagging_jobs", |limit| {
        db.active_tagging_jobs(limit).unwrap().len()
    });
    assert_limit_contract("active_tagging_jobs_for_ui", |limit| {
        db.active_tagging_jobs_for_ui(limit).unwrap().len()
    });
}

#[test]
fn prune_send_history_zero_stays_an_explicit_clear_all() {
    let db = Db::open_in_memory().unwrap();
    for _ in 0..3 {
        db.connection()
            .execute(
                "INSERT INTO send_outbox(photo_id, kind, origin, status, created_at) \
                     VALUES (NULL, 'test', 'manual', 'sent', ?1)",
                params!["2026-11-06T00:00:00+00:00"],
            )
            .unwrap();
    }
    assert_eq!(db.prune_send_history(0).unwrap(), 3);
    assert!(db.send_history(10).unwrap().is_empty());
}

#[test]
fn text_search_paginates_and_counts_the_same_filter() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    for (name, captured) in [("a.png", 100), ("b.png", 200), ("c.png", 300)] {
        db.upsert_photo(&sample_photo(root, &format!("/photos/{name}"), captured))
            .unwrap();
    }
    let filter = PhotoTextFilter::default();
    assert_eq!(db.count_photos_text(&filter).unwrap(), 3);
    let page = db.search_photos_text(&filter, 1, 1).unwrap();
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].file_name, "b.png");
}

#[test]
fn text_search_applies_tag_favorite_and_date_filters() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let old = db
        .upsert_photo(&sample_photo(root, "/photos/old.png", 100))
        .unwrap();
    let new = db
        .upsert_photo(&sample_photo(root, "/photos/new.png", 300))
        .unwrap();
    db.set_photo_tags(old, &["beach".to_string()]).unwrap();
    db.set_favorite(new, true).unwrap();

    let tag_filter = PhotoTextFilter {
        tag: Some("beach".to_string()),
        ..PhotoTextFilter::default()
    };
    let rows = db.search_photos_text(&tag_filter, 10, 0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, old);

    let favorite_filter = PhotoTextFilter {
        favorite: true,
        from_utc: Some(200),
        ..PhotoTextFilter::default()
    };
    let rows = db.search_photos_text(&favorite_filter, 10, 0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, new);
}

#[test]
fn worlds_with_photo_counts_group_visible_photos_and_skip_missing() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let a = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();
    let b = db
        .upsert_photo(&sample_photo(root, "/photos/b.png", 200))
        .unwrap();
    let c = db
        .upsert_photo(&sample_photo(root, "/photos/c.png", 300))
        .unwrap();
    let missing = db
        .upsert_photo(&sample_photo(root, "/photos/missing.png", 400))
        .unwrap();
    let world_a = db
        .upsert_world("wrld_a", Some("World A"), Some("log"))
        .unwrap();
    let world_b = db
        .upsert_world("wrld_b", Some("World B"), Some("log"))
        .unwrap();
    db.connection()
        .execute(
            "UPDATE worlds SET alias = 'Alias A' WHERE id = ?1",
            params![world_a],
        )
        .unwrap();
    for (photo_id, world_id, name) in [
        (a, world_a, "World A"),
        (b, world_a, "World A"),
        (c, world_b, "World B"),
        (missing, world_a, "World A"),
    ] {
        db.set_photo_context(&PhotoContext {
            photo_id,
            session_id: None,
            world_id: Some(world_id),
            world_name: Some(name.to_string()),
            instance_id: None,
            link_method: "log".to_string(),
            confidence: 1.0,
            author: None,
            linked_at: "2026-11-09T00:00:00+00:00".to_string(),
        })
        .unwrap();
    }
    db.mark_missing(missing).unwrap();

    let worlds = db.worlds_with_photo_counts().unwrap();
    assert_eq!(
        worlds,
        vec![
            WorldPhotoCount {
                world_id: Some("wrld_a".to_string()),
                name: "Alias A".to_string(),
                photo_count: 2,
            },
            WorldPhotoCount {
                world_id: Some("wrld_b".to_string()),
                name: "World B".to_string(),
                photo_count: 1,
            },
        ]
    );

    let stats = db.stats().unwrap();
    assert_eq!(stats.photo_count, 3);
    assert_eq!(stats.world_count, 2);
}

#[test]
fn random_photos_text_is_deterministic_for_a_seed() {
    let db = Db::open_in_memory().unwrap();
    seed_photos(&db, 20);
    let run = || {
        db.random_photos_text(&PhotoTextFilter::default(), 5, 42)
            .unwrap()
            .photos
            .into_iter()
            .map(|photo| photo.id)
            .collect::<Vec<_>>()
    };
    let first = run();
    assert_eq!(first.len(), 5);
    assert_eq!(first, run());
    let mut unique = first;
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 5);
}

#[test]
fn random_photos_text_varies_with_the_seed() {
    let db = Db::open_in_memory().unwrap();
    seed_photos(&db, 20);
    let sample = |seed| {
        db.random_photos_text(&PhotoTextFilter::default(), 5, seed)
            .unwrap()
            .photos
            .into_iter()
            .map(|photo| photo.id)
            .collect::<Vec<_>>()
    };
    let results: Vec<_> = (0..8).map(sample).collect();
    assert!(
        results.windows(2).any(|pair| pair[0] != pair[1]),
        "different seeds must produce different samples: {results:?}"
    );
}

#[test]
fn random_photos_text_returns_all_matches_when_the_limit_is_larger() {
    let db = Db::open_in_memory().unwrap();
    let ids = seed_photos(&db, 3);
    let sample = db
        .random_photos_text(&PhotoTextFilter::default(), 10, 1)
        .unwrap();
    assert_eq!(sample.total, 3);
    assert_eq!(sample.photos.len(), 3);
    let mut got: Vec<_> = sample.photos.iter().map(|photo| photo.id).collect();
    got.sort_unstable();
    assert_eq!(got, ids);

    let empty_db = Db::open_in_memory().unwrap();
    let empty = empty_db
        .random_photos_text(&PhotoTextFilter::default(), 10, 0)
        .unwrap();
    assert_eq!(empty.total, 0);
    assert!(empty.photos.is_empty());
}

#[test]
fn random_photos_text_honors_filters() {
    let db = Db::open_in_memory().unwrap();
    let ids = seed_photos(&db, 5);
    db.set_favorite(ids[2], true).unwrap();
    let sample = db
        .random_photos_text(
            &PhotoTextFilter {
                favorite: true,
                ..Default::default()
            },
            10,
            99,
        )
        .unwrap();
    assert_eq!(sample.total, 1);
    assert_eq!(sample.photos.len(), 1);
    assert_eq!(sample.photos[0].id, ids[2]);
}

#[test]
fn photo_index_sql_uses_stored_month_and_day() {
    let sql = photo_index_sql(PhotoSort::Newest);
    assert!(!sql.contains("strftime"));
    assert!(sql.contains("p.captured_month"));
    assert!(sql.contains("p.captured_day"));
}

#[test]
fn generated_month_and_day_match_the_legacy_formula() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    db.upsert_photo(&photo_at(
        root,
        "/photos/a.png",
        1_700_000_000,
        "2026-11-11 12:00:00.000",
    ))
    .unwrap();
    db.upsert_photo(&photo_at(
        root,
        "/photos/b.png",
        1_700_000_000,
        "not-a-date",
    ))
    .unwrap();

    let mut stmt = db
        .connection()
        .prepare(
            "SELECT captured_month, \
                    COALESCE(\
                        CAST(strftime('%Y%m', captured_at_local) AS INTEGER), \
                        CAST(strftime('%Y%m', captured_at_utc, 'unixepoch') AS INTEGER), \
                        0), \
                 captured_day, \
                    COALESCE(\
                        CAST(strftime('%Y%m%d', captured_at_local) AS INTEGER), \
                        CAST(strftime('%Y%m%d', captured_at_utc, 'unixepoch') AS INTEGER), \
                        0) \
                 FROM photos ORDER BY id",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(rows.len(), 2);
    for (month, legacy_month, day, legacy_day) in rows {
        assert_eq!(month, legacy_month);
        assert_eq!(day, legacy_day);
    }
}

#[test]
fn bundled_sqlite_supports_strftime_in_stored_generated_columns() {
    let db = Db::open_in_memory().unwrap();
    db.connection()
        .execute_batch(
            "CREATE TABLE probe (
                    local TEXT NOT NULL,
                    utc INTEGER NOT NULL,
                    month INTEGER GENERATED ALWAYS AS (
                        COALESCE(
                            CAST(strftime('%Y%m', local) AS INTEGER),
                            CAST(strftime('%Y%m', utc, 'unixepoch') AS INTEGER),
                            0
                        )
                    ) STORED
                );
                INSERT INTO probe(local, utc) VALUES ('2026-11-11 12:00:00.000', 0);
                INSERT INTO probe(local, utc) VALUES ('not-a-date', 0);",
        )
        .unwrap();
    let month: i64 = db
        .connection()
        .query_row(
            "SELECT month FROM probe WHERE local LIKE '2026%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(month, 202611);
}

#[test]
fn fresh_database_is_initialized_to_v19() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let db = Db::open(&path).unwrap();

    let version: i64 = db
        .connection()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 19);

    let tables: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' \
             AND name IN ('photos', 'library_roots', 'photo_ai', 'send_outbox')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 4);

    let generated: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_xinfo('photos') \
             WHERE name IN ('captured_month', 'captured_day')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(generated, 2);

    let migrations_table: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master \
             WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(migrations_table, 0);
}

#[test]
fn existing_v19_database_opens_without_reinitializing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let db = Db::open(&path).unwrap();
    db.connection()
        .execute(
            "INSERT INTO app_settings(key, value) VALUES ('marker', 'keep') \
             ON CONFLICT(key) DO UPDATE SET value = 'keep'",
            [],
        )
        .unwrap();
    drop(db);

    let db = Db::open(&path).unwrap();
    let value: String = db
        .connection()
        .query_row(
            "SELECT value FROM app_settings WHERE key = 'marker'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(value, "keep");
}

#[test]
fn v18_database_is_rejected_without_modification() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE legacy_marker(value TEXT); \
             INSERT INTO legacy_marker(value) VALUES ('keep');",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 18).unwrap();
    }
    let before = std::fs::read(&path).unwrap();

    let error = match Db::open(&path) {
        Ok(_) => panic!("expected the v18 database to be rejected"),
        Err(error) => error,
    };
    let CoreError::Invalid(message) = error else {
        panic!("expected an explicit schema error, got {error:?}");
    };
    assert!(message.contains("18"), "{message}");
    assert!(message.contains("cannot be migrated"), "{message}");

    let after = std::fs::read(&path).unwrap();
    assert_eq!(before, after, "the rejected database must not be modified");

    let conn = Connection::open(&path).unwrap();
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 18);
    let marker: String = conn
        .query_row("SELECT value FROM legacy_marker", [], |row| row.get(0))
        .unwrap();
    assert_eq!(marker, "keep");
}

#[test]
fn compression_job_origin_roundtrips() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();

    db.enqueue_job(photo, "p", "keep").unwrap();
    db.enqueue_auto_job_with_status(photo, "keep", QueueStatus::Pending)
        .unwrap();

    let jobs = db.list_jobs(10).unwrap();
    assert_eq!(jobs.len(), 2);
    assert!(
        jobs.iter()
            .any(|job| job.origin == CompressionOrigin::Manual
                && job.profile_id.as_deref() == Some("p"))
    );
    assert!(
        jobs.iter()
            .any(|job| job.origin == CompressionOrigin::Auto && job.profile_id.is_none())
    );
}

#[test]
fn compression_job_profile_id_is_nullable_in_the_schema() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();

    db.connection()
        .execute(
            "INSERT INTO compression_jobs(id, photo_id, profile_id, origin, status, progress, policy, created_at) \
             VALUES ('auto-null', ?1, NULL, 'auto', 'pending', 0, 'keep', '2026-01-01T00:00:00Z')",
            params![photo],
        )
        .unwrap();

    let job = db
        .list_jobs(1)
        .unwrap()
        .into_iter()
        .find(|job| job.id == "auto-null")
        .unwrap();
    assert_eq!(job.origin, CompressionOrigin::Auto);
    assert_eq!(job.profile_id, None);
}

#[test]
fn derivative_origin_and_nullable_profile_roundtrip() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let photo = db
        .upsert_photo(&sample_photo(root, "/photos/a.png", 100))
        .unwrap();

    db.add_derivative(
        photo,
        "/photos/compressed/auto.webp",
        "webp",
        16,
        10,
        10,
        None,
        CompressionOrigin::Auto,
        "replace",
    )
    .unwrap();
    db.add_derivative(
        photo,
        "/photos/compressed/manual.webp",
        "webp",
        16,
        10,
        11,
        Some("p"),
        CompressionOrigin::Manual,
        "keep",
    )
    .unwrap();

    let derivatives = db.derivatives_for_photo(photo).unwrap();
    assert_eq!(derivatives.len(), 2);
    assert!(
        derivatives.iter().any(|derivative| derivative.origin
            == CompressionOrigin::Auto
            && derivative.profile_id.is_none())
    );
    assert!(
        derivatives.iter().any(|derivative| derivative.origin
            == CompressionOrigin::Manual
            && derivative.profile_id.as_deref() == Some("p"))
    );
}

#[test]
fn photos_without_hash_after_skips_earlier_ids() {
    let db = Db::open_in_memory().unwrap();
    let root = db.add_root("/photos", "reference").unwrap();
    let mut ids = Vec::new();
    for name in ["a.png", "b.png", "c.png"] {
        ids.push(
            db.upsert_photo(&sample_photo(
                root,
                &format!("/photos/{name}"),
                1_700_000_000,
            ))
            .unwrap(),
        );
    }

    let first_page: Vec<i64> = db
        .photos_without_hash_after(2, 0)
        .unwrap()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(first_page, ids[..2]);

    let second_page: Vec<i64> = db
        .photos_without_hash_after(2, ids[1])
        .unwrap()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(second_page, vec![ids[2]]);

    assert!(db.photos_without_hash_after(2, ids[2]).unwrap().is_empty());
}

#[test]
fn has_daemon_commands_reports_only_queued_rows() {
    let db = Db::open_in_memory().unwrap();
    assert!(!db.has_daemon_commands().unwrap());

    db.request_daemon_command(DaemonCommand::Rescan).unwrap();
    assert!(db.has_daemon_commands().unwrap());
    assert_eq!(
        db.take_daemon_commands().unwrap(),
        vec![DaemonCommand::Rescan]
    );
    assert!(!db.has_daemon_commands().unwrap());
}

#[test]
fn data_version_changes_after_another_connection_commits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("library.db");
    let reader = Db::open(&path).unwrap();
    let writer = Db::open(&path).unwrap();

    let before = reader.data_version().unwrap();
    writer.set_setting("version_probe", "1").unwrap();
    assert!(reader.data_version().unwrap() > before);
}
