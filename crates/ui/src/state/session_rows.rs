//! Session-aware display row model for the Photos grid.
//!
//! The virtual list needs one entry per header, date label, or photo row.
//! This module owns only the pure row construction and scroll-position maths;
//! rendering and loading stay in the UI and browser layers.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use vrc_photon_core::model::PhotoIndexEntry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionRowKind {
    Header {
        session_id: i64,
        photo_count: usize,
        collapsed: bool,
    },
    Photos {
        first_photo: usize,
        count: usize,
    },
    DateLabel {
        day: i32,
        photo_count: usize,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SessionRow {
    pub(crate) kind: SessionRowKind,
}

impl SessionRow {
    pub(crate) fn kind(&self) -> SessionRowKind {
        self.kind
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PhotoRowSpan {
    first_photo: usize,
    count: usize,
    row: usize,
}

pub(crate) struct SessionRows {
    rows: Vec<SessionRow>,
    heights: Vec<f32>,
    origins: Vec<f32>,
    photo_rows: Vec<PhotoRowSpan>,
    /// First photo represented by each row. A header or date label maps to the
    /// photos below it, so the gallery header can describe the top row even
    /// when the top row is not itself a photo row.
    row_photo_start: Vec<usize>,
    header_rows: HashMap<i64, usize>,
    /// First photo of each header or date-label row. Anchoring to one of
    /// these photos keeps its context row visible.
    leading_rows: HashMap<usize, usize>,
    /// Photo range hidden by each collapsed session, with its header row.
    hidden_session_ranges: Vec<(usize, usize, usize)>,
    /// Row ranges carrying each session's photos. A session that is split by
    /// unlinked photos has more than one range.
    session_photo_rows: HashMap<i64, Vec<Range<usize>>>,
    photo_count: usize,
}

impl SessionRows {
    pub(crate) fn build(
        entries: &[PhotoIndexEntry],
        columns: usize,
        photo_row_height: f32,
        header_height: f32,
        date_label_height: f32,
        collapsed: &HashSet<i64>,
        animating: &HashSet<i64>,
    ) -> Self {
        let columns = columns.max(1);
        let mut total_counts: HashMap<i64, usize> = HashMap::new();
        for entry in entries {
            if let Some(session_id) = entry.session_id {
                *total_counts.entry(session_id).or_insert(0) += 1;
            }
        }

        let mut rows = Vec::new();
        let mut heights = Vec::new();
        let mut photo_rows = Vec::new();
        let mut row_photo_start = Vec::new();
        let mut header_rows = HashMap::new();
        let mut leading_rows = HashMap::new();
        let mut hidden_session_ranges = Vec::new();
        let mut session_photo_rows: HashMap<i64, Vec<Range<usize>>> = HashMap::new();
        let mut seen_sessions = HashSet::new();
        let mut index = 0;

        while index < entries.len() {
            if let Some(session_id) = entries[index].session_id {
                let start = index;
                while index < entries.len() && entries[index].session_id == Some(session_id) {
                    index += 1;
                }
                let run_len = index - start;
                let is_collapsed = collapsed.contains(&session_id);
                // A session whose photo run was interrupted by unlinked
                // photos keeps only its first header (decision Q6).
                if seen_sessions.insert(session_id) {
                    let row = rows.len();
                    rows.push(SessionRow {
                        kind: SessionRowKind::Header {
                            session_id,
                            photo_count: total_counts.get(&session_id).copied().unwrap_or(run_len),
                            collapsed: is_collapsed,
                        },
                    });
                    heights.push(header_height);
                    row_photo_start.push(start);
                    header_rows.insert(session_id, row);
                    leading_rows.insert(start, row);
                }
                if is_collapsed && !animating.contains(&session_id) {
                    if let Some(header_row) = header_rows.get(&session_id) {
                        hidden_session_ranges.push((start, index, *header_row));
                    }
                } else {
                    let first_row = rows.len();
                    push_photo_rows(
                        &mut rows,
                        &mut heights,
                        &mut photo_rows,
                        &mut row_photo_start,
                        start,
                        run_len,
                        columns,
                        photo_row_height,
                    );
                    let end_row = rows.len();
                    if first_row < end_row {
                        session_photo_rows
                            .entry(session_id)
                            .or_default()
                            .push(first_row..end_row);
                    }
                }
            } else {
                let start = index;
                while index < entries.len() && entries[index].session_id.is_none() {
                    index += 1;
                }
                let mut day_start = start;
                while day_start < index {
                    let day = entries[day_start].captured_day;
                    let mut day_end = day_start + 1;
                    while day_end < index && entries[day_end].captured_day == day {
                        day_end += 1;
                    }
                    let count = day_end - day_start;
                    rows.push(SessionRow {
                        kind: SessionRowKind::DateLabel {
                            day,
                            photo_count: count,
                        },
                    });
                    heights.push(date_label_height);
                    row_photo_start.push(day_start);
                    leading_rows.insert(day_start, rows.len() - 1);
                    push_photo_rows(
                        &mut rows,
                        &mut heights,
                        &mut photo_rows,
                        &mut row_photo_start,
                        day_start,
                        count,
                        columns,
                        photo_row_height,
                    );
                    day_start = day_end;
                }
            }
        }

        let mut origins = Vec::with_capacity(heights.len());
        let mut offset = 0.0;
        for height in &heights {
            origins.push(offset);
            offset += *height;
        }

        Self {
            rows,
            heights,
            origins,
            photo_rows,
            row_photo_start,
            header_rows,
            leading_rows,
            hidden_session_ranges,
            session_photo_rows,
            photo_count: entries.len(),
        }
    }

    pub(crate) fn rows(&self) -> &[SessionRow] {
        &self.rows
    }

    pub(crate) fn heights(&self) -> &[f32] {
        &self.heights
    }

    /// Row heights after applying per-session collapse progress. `0.0` leaves
    /// the session expanded and `1.0` collapses its photo rows to zero.
    pub(crate) fn heights_with_collapse(&self, factors: &HashMap<i64, f32>) -> Vec<f32> {
        if factors.is_empty() {
            return self.heights.clone();
        }
        let mut heights = self.heights.clone();
        for (session_id, factor) in factors {
            let Some(ranges) = self.session_photo_rows.get(session_id) else {
                continue;
            };
            let visible = 1.0 - factor.clamp(0.0, 1.0);
            for range in ranges {
                for height in &mut heights[range.clone()] {
                    *height *= visible;
                }
            }
        }
        heights
    }

    /// Photo shown at the top of the viewport for a non-negative scroll
    /// offset. Header and date-label rows resolve to the photos below them.
    pub(crate) fn top_photo_index(&self, scroll_top: f32) -> Option<usize> {
        self.top_photo_index_with_origins(scroll_top, &self.origins)
    }

    /// Cumulative origins for a caller-supplied height vector. Used while a
    /// collapse animation is painting heights that differ from the settled
    /// row model.
    pub(crate) fn origins_with_heights(&self, heights: &[f32]) -> Vec<f32> {
        let mut origins = Vec::with_capacity(heights.len());
        let mut offset = 0.0;
        for height in heights {
            origins.push(offset);
            offset += *height;
        }
        origins
    }

    /// Photo shown at the top of the viewport using animated row heights.
    pub(crate) fn top_photo_index_with_heights(
        &self,
        scroll_top: f32,
        heights: &[f32],
    ) -> Option<usize> {
        self.top_photo_index_with_origins(scroll_top, &self.origins_with_heights(heights))
    }

    fn top_photo_index_with_origins(&self, scroll_top: f32, origins: &[f32]) -> Option<usize> {
        if self.rows.is_empty() {
            return None;
        }
        let top = scroll_top.max(0.0);
        let index = origins
            .partition_point(|origin| *origin <= top)
            .saturating_sub(1);
        let photo = *self.row_photo_start.get(index)?;
        (photo < self.photo_count).then_some(photo)
    }

    /// Offset of the photo row that contains `photo_index`. Returns `None`
    /// when the photo is hidden inside a collapsed session.
    pub(crate) fn offset_for_photo(&self, photo_index: usize) -> Option<f32> {
        self.row_for_photo(photo_index).map(|row| self.origins[row])
    }

    #[cfg(test)]
    pub(crate) fn offset_for_session(&self, session_id: i64) -> Option<f32> {
        self.header_rows
            .get(&session_id)
            .map(|row| self.origins[*row])
    }

    /// Header offset using a caller-supplied height vector, so scrolling can
    /// follow the same animated geometry the virtual list is painting.
    pub(crate) fn offset_for_session_with_heights(
        &self,
        session_id: i64,
        heights: &[f32],
    ) -> Option<f32> {
        let row = *self.header_rows.get(&session_id)?;
        Some(heights.get(..row)?.iter().sum())
    }

    /// Offset for anchoring `photo_index` at the top of the viewport. When
    /// the photo starts a session or date-label run, keep that context row
    /// visible; otherwise anchor to the photo row itself.
    pub(crate) fn anchor_offset_for_photo(&self, photo_index: usize) -> Option<f32> {
        if let Some(row) = self.leading_rows.get(&photo_index) {
            return Some(self.origins[*row]);
        }
        if let Some(offset) = self.offset_for_photo(photo_index) {
            return Some(offset);
        }
        let index = self
            .hidden_session_ranges
            .partition_point(|(start, _, _)| *start <= photo_index);
        let (_, end, row) = self.hidden_session_ranges.get(index.checked_sub(1)?)?;
        (photo_index < *end).then_some(self.origins[*row])
    }

    pub(crate) fn row_for_photo(&self, photo_index: usize) -> Option<usize> {
        if photo_index >= self.photo_count {
            return None;
        }
        let index = self
            .photo_rows
            .partition_point(|span| span.first_photo <= photo_index);
        let span = self.photo_rows.get(index.checked_sub(1)?)?;
        (photo_index < span.first_photo + span.count).then_some(span.row)
    }
}

#[allow(clippy::too_many_arguments)]
fn push_photo_rows(
    rows: &mut Vec<SessionRow>,
    heights: &mut Vec<f32>,
    photo_rows: &mut Vec<PhotoRowSpan>,
    row_photo_start: &mut Vec<usize>,
    start: usize,
    count: usize,
    columns: usize,
    height: f32,
) {
    let mut offset = 0;
    while offset < count {
        let row_count = columns.min(count - offset);
        let row = rows.len();
        let first_photo = start + offset;
        rows.push(SessionRow {
            kind: SessionRowKind::Photos {
                first_photo,
                count: row_count,
            },
        });
        heights.push(height);
        row_photo_start.push(first_photo);
        photo_rows.push(PhotoRowSpan {
            first_photo,
            count: row_count,
            row,
        });
        offset += row_count;
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionRowKind, SessionRows};
    use std::collections::{HashMap, HashSet};
    use vrc_photon_core::model::PhotoIndexEntry;

    fn entry(id: i64, day: i32, session: Option<i64>) -> PhotoIndexEntry {
        PhotoIndexEntry {
            id,
            captured_at_utc: id,
            captured_month: day / 100,
            captured_day: day,
            session_id: session,
        }
    }

    #[test]
    fn groups_contiguous_session_photos_behind_one_header() {
        let entries = vec![
            entry(5, 20261109, Some(7)),
            entry(4, 20261109, Some(7)),
            entry(3, 20261108, Some(8)),
            entry(2, 20261108, Some(8)),
        ];
        let rows = SessionRows::build(
            &entries,
            2,
            200.0,
            92.0,
            46.0,
            &HashSet::new(),
            &HashSet::new(),
        );
        assert_eq!(
            rows.rows().iter().map(|row| row.kind()).collect::<Vec<_>>(),
            vec![
                SessionRowKind::Header {
                    session_id: 7,
                    photo_count: 2,
                    collapsed: false
                },
                SessionRowKind::Photos {
                    first_photo: 0,
                    count: 2
                },
                SessionRowKind::Header {
                    session_id: 8,
                    photo_count: 2,
                    collapsed: false
                },
                SessionRowKind::Photos {
                    first_photo: 2,
                    count: 2
                },
            ]
        );
        assert_eq!(rows.heights(), &[92.0, 200.0, 92.0, 200.0]);
    }

    #[test]
    fn splits_unlinked_runs_by_local_day_and_never_groups_them_as_sessions() {
        let entries = vec![
            entry(6, 20261109, None),
            entry(5, 20261109, None),
            entry(4, 20261108, None),
            entry(3, 20261108, None),
            entry(2, 20261108, Some(3)),
        ];
        let rows = SessionRows::build(
            &entries,
            2,
            200.0,
            92.0,
            46.0,
            &HashSet::new(),
            &HashSet::new(),
        );
        assert_eq!(
            rows.rows().iter().map(|row| row.kind()).collect::<Vec<_>>(),
            vec![
                SessionRowKind::DateLabel {
                    day: 20261109,
                    photo_count: 2
                },
                SessionRowKind::Photos {
                    first_photo: 0,
                    count: 2
                },
                SessionRowKind::DateLabel {
                    day: 20261108,
                    photo_count: 2
                },
                SessionRowKind::Photos {
                    first_photo: 2,
                    count: 2
                },
                SessionRowKind::Header {
                    session_id: 3,
                    photo_count: 1,
                    collapsed: false
                },
                SessionRowKind::Photos {
                    first_photo: 4,
                    count: 1
                },
            ]
        );
    }

    #[test]
    fn collapsed_sessions_keep_the_header_and_drop_photo_rows() {
        let entries = vec![
            entry(4, 20261109, Some(7)),
            entry(3, 20261109, Some(7)),
            entry(2, 20261108, Some(8)),
        ];
        let collapsed = HashSet::from([7]);
        let rows = SessionRows::build(&entries, 2, 200.0, 92.0, 46.0, &collapsed, &HashSet::new());
        assert_eq!(
            rows.rows().iter().map(|row| row.kind()).collect::<Vec<_>>(),
            vec![
                SessionRowKind::Header {
                    session_id: 7,
                    photo_count: 2,
                    collapsed: true
                },
                SessionRowKind::Header {
                    session_id: 8,
                    photo_count: 1,
                    collapsed: false
                },
                SessionRowKind::Photos {
                    first_photo: 2,
                    count: 1
                },
            ]
        );
        assert!(rows.offset_for_photo(0).is_none());
        assert!(rows.offset_for_photo(1).is_none());
        assert_eq!(rows.anchor_offset_for_photo(0), rows.offset_for_session(7));
        assert_eq!(rows.anchor_offset_for_photo(1), rows.offset_for_session(7));
    }

    #[test]
    fn a_split_session_keeps_only_its_first_header() {
        let entries = vec![
            entry(4, 20261109, Some(7)),
            entry(3, 20261109, Some(7)),
            entry(2, 20261109, None),
            entry(1, 20261109, Some(7)),
        ];
        let rows = SessionRows::build(
            &entries,
            2,
            200.0,
            92.0,
            46.0,
            &HashSet::new(),
            &HashSet::new(),
        );
        assert_eq!(
            rows.rows().iter().map(|row| row.kind()).collect::<Vec<_>>(),
            vec![
                SessionRowKind::Header {
                    session_id: 7,
                    photo_count: 3,
                    collapsed: false
                },
                SessionRowKind::Photos {
                    first_photo: 0,
                    count: 2
                },
                SessionRowKind::DateLabel {
                    day: 20261109,
                    photo_count: 1
                },
                SessionRowKind::Photos {
                    first_photo: 2,
                    count: 1
                },
                SessionRowKind::Photos {
                    first_photo: 3,
                    count: 1
                },
            ]
        );
    }

    #[test]
    fn offsets_and_top_photo_index_round_trip_across_headers() {
        let entries: Vec<_> = (0..10)
            .map(|index| entry(10 - index, 20261109, Some(index / 2)))
            .collect();
        let rows = SessionRows::build(
            &entries,
            2,
            200.0,
            92.0,
            46.0,
            &HashSet::new(),
            &HashSet::new(),
        );
        for photo in (0..10).step_by(2) {
            let offset = rows.offset_for_photo(photo).unwrap();
            assert_eq!(rows.top_photo_index(offset), Some(photo));
        }
        // A header row maps to the photo below it.
        let header_offset = rows.offset_for_session(0).unwrap();
        assert_eq!(rows.top_photo_index(header_offset), Some(0));
        assert_eq!(rows.top_photo_index(-10.0), Some(0));
    }

    #[test]
    fn anchor_offset_keeps_the_context_row_for_the_first_photo() {
        let entries = vec![
            entry(4, 20261109, Some(7)),
            entry(3, 20261109, Some(7)),
            entry(2, 20261109, None),
            entry(1, 20261109, None),
        ];
        let rows = SessionRows::build(
            &entries,
            2,
            200.0,
            92.0,
            46.0,
            &HashSet::new(),
            &HashSet::new(),
        );
        assert_eq!(rows.anchor_offset_for_photo(0), rows.offset_for_session(7));
        assert_eq!(rows.anchor_offset_for_photo(1), rows.offset_for_photo(1));
        assert!(rows.anchor_offset_for_photo(2).unwrap() < rows.offset_for_photo(2).unwrap());
        assert_eq!(rows.anchor_offset_for_photo(3), rows.offset_for_photo(3));
    }

    #[test]
    fn row_lookup_handles_unknown_and_out_of_range_photos() {
        let entries = vec![entry(1, 20261109, Some(1))];
        let rows = SessionRows::build(
            &entries,
            2,
            200.0,
            92.0,
            46.0,
            &HashSet::new(),
            &HashSet::new(),
        );
        assert_eq!(rows.row_for_photo(0), Some(1));
        assert_eq!(rows.row_for_photo(1), None);
        assert_eq!(rows.offset_for_session(99), None);
        assert_eq!(rows.offset_for_photo(99), None);
    }

    #[test]
    fn an_animating_collapsed_session_keeps_its_photo_rows() {
        let entries = vec![entry(4, 20261109, Some(7)), entry(3, 20261109, Some(7))];
        let rows = SessionRows::build(
            &entries,
            2,
            200.0,
            92.0,
            46.0,
            &HashSet::from([7]),
            &HashSet::from([7]),
        );
        assert_eq!(
            rows.rows().iter().map(|row| row.kind()).collect::<Vec<_>>(),
            vec![
                SessionRowKind::Header {
                    session_id: 7,
                    photo_count: 2,
                    collapsed: true
                },
                SessionRowKind::Photos {
                    first_photo: 0,
                    count: 2
                },
            ]
        );
        assert_eq!(rows.offset_for_photo(0), Some(92.0));
    }

    #[test]
    fn heights_with_collapse_scales_only_the_matching_session_rows() {
        let entries = vec![
            entry(6, 20261109, Some(7)),
            entry(5, 20261109, Some(7)),
            entry(4, 20261108, Some(8)),
            entry(3, 20261108, Some(8)),
        ];
        let rows = SessionRows::build(
            &entries,
            2,
            200.0,
            92.0,
            46.0,
            &HashSet::new(),
            &HashSet::new(),
        );
        let heights = rows.heights_with_collapse(&HashMap::from([(7, 0.5)]));
        assert_eq!(heights, vec![92.0, 100.0, 92.0, 200.0]);

        let collapsed = rows.heights_with_collapse(&HashMap::from([(7, 1.0)]));
        assert_eq!(collapsed, vec![92.0, 0.0, 92.0, 200.0]);

        let expanded = rows.heights_with_collapse(&HashMap::from([(7, 0.0)]));
        assert_eq!(expanded, rows.heights());
    }

    #[test]
    fn a_split_session_scales_every_photo_range() {
        let entries = vec![
            entry(4, 20261109, Some(7)),
            entry(3, 20261109, Some(7)),
            entry(2, 20261109, None),
            entry(1, 20261109, Some(7)),
        ];
        let rows = SessionRows::build(
            &entries,
            1,
            100.0,
            40.0,
            20.0,
            &HashSet::new(),
            &HashSet::new(),
        );
        let heights = rows.heights_with_collapse(&HashMap::from([(7, 0.25)]));
        // Header 40, first run 2 photos, date label 20, unlinked photo 1,
        // split session photo 1.
        assert_eq!(heights, vec![40.0, 75.0, 75.0, 20.0, 100.0, 75.0]);
    }

    #[test]
    fn heights_with_collapse_ignores_unknown_sessions_and_clamps() {
        let rows = SessionRows::build(
            &[entry(1, 20261109, Some(7))],
            1,
            100.0,
            40.0,
            20.0,
            &HashSet::new(),
            &HashSet::new(),
        );
        assert_eq!(
            rows.heights_with_collapse(&HashMap::from([(99, 0.5)])),
            rows.heights()
        );
        assert_eq!(
            rows.heights_with_collapse(&HashMap::from([(7, 2.0)])),
            vec![40.0, 0.0]
        );
    }

    #[test]
    fn top_photo_index_follows_the_animated_heights() {
        let entries = vec![
            entry(6, 20261109, Some(7)),
            entry(5, 20261109, Some(7)),
            entry(4, 20261108, Some(8)),
            entry(3, 20261108, Some(8)),
        ];
        let rows = SessionRows::build(
            &entries,
            2,
            200.0,
            92.0,
            46.0,
            &HashSet::new(),
            &HashSet::new(),
        );
        let animated = rows.heights_with_collapse(&HashMap::from([(7, 0.5)]));
        // Header 92 + half photo row 100 => the second header starts at 192.
        assert_eq!(rows.top_photo_index_with_heights(200.0, &animated), Some(2));
        // The settled geometry still has the first photo row at 200..400.
        assert_eq!(rows.top_photo_index(200.0), Some(0));
    }

    #[test]
    fn session_offset_can_use_animated_heights() {
        let entries = vec![
            entry(6, 20261109, Some(7)),
            entry(5, 20261109, Some(7)),
            entry(4, 20261108, Some(8)),
            entry(3, 20261108, Some(8)),
        ];
        let rows = SessionRows::build(
            &entries,
            2,
            200.0,
            92.0,
            46.0,
            &HashSet::new(),
            &HashSet::new(),
        );
        let animated = rows.heights_with_collapse(&HashMap::from([(7, 0.5)]));
        assert_eq!(
            rows.offset_for_session_with_heights(8, &animated),
            Some(192.0)
        );
        assert_eq!(rows.offset_for_session_with_heights(99, &animated), None);
    }
}
