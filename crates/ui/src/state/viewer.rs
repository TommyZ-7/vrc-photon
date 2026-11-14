//! Viewer open/close state and the morph timelines used by Quick Look.
//!
//! Screen painting and `App` methods stay in the UI layer; this module owns
//! the state values and the pure timeline helpers they share.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::{Bounds, Pixels, Point, Size, point};

use crate::state::full_image::FullImageTexture;
use crate::state::thumbnail::ThumbnailHandle;

use crate::ui::theme::{
    MOTION_BASE, MOTION_MORPH, VIEWER_MINIMAP_HEIGHT, VIEWER_MINIMAP_WIDTH, eased_progress,
    lerp_bounds, viewer_image_box_with_details_width,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ViewerOpenSource {
    Card,
    Inspector,
    Auto,
}

/// Chooses the corner-radius sweep for one morph.
pub(crate) fn morph_radius_plan(closing: bool, source: ViewerOpenSource) -> (f32, f32) {
    let inspector_source = source == ViewerOpenSource::Inspector;
    if closing {
        (10.0, if inspector_source { 11.0 } else { 12.0 })
    } else {
        (if inspector_source { 11.0 } else { 12.0 }, 10.0)
    }
}

pub(crate) const ZOOM_STEP: f32 = 1.25;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ViewerZoom {
    /// Scale relative to the fit box: 1.0 is fit, 1 / ratio is 100%.
    pub(crate) scale: f32,
    /// Screen-pixel offset from the centered position.
    pub(crate) pan: Point<f32>,
}

impl Default for ViewerZoom {
    fn default() -> Self {
        Self {
            scale: 1.0,
            pan: Point::default(),
        }
    }
}

impl ViewerZoom {
    pub(crate) fn is_active(self) -> bool {
        self.scale > 1.0 + 1e-4
    }

    pub(crate) fn percent(self, image: (u32, u32), frame: Size<f32>) -> u32 {
        (self.scale * pixel_ratio(image, frame) * 100.0).round() as u32
    }

    pub(crate) fn clamped(self, image: (u32, u32), frame: Size<f32>) -> Self {
        let ratio = pixel_ratio(image, frame);
        let (min, max) = zoom_limits(ratio);
        let scale = self.scale.clamp(min, max);
        let max_pan_x = (frame.width * (scale - 1.0) / 2.0).max(0.0);
        let max_pan_y = (frame.height * (scale - 1.0) / 2.0).max(0.0);
        Self {
            scale,
            pan: point(
                self.pan.x.clamp(-max_pan_x, max_pan_x),
                self.pan.y.clamp(-max_pan_y, max_pan_y),
            ),
        }
    }

    pub(crate) fn zoom_at(
        self,
        cursor: Point<f32>,
        factor: f32,
        image: (u32, u32),
        frame: Size<f32>,
    ) -> Self {
        let ratio = pixel_ratio(image, frame);
        let (min, max) = zoom_limits(ratio);
        let scale = (self.scale * factor).clamp(min, max);
        if (scale - self.scale).abs() < 1e-6 {
            return self;
        }
        let change = scale / self.scale;
        let pan = cursor - (cursor - self.pan) * change;
        Self { scale, pan }.clamped(image, frame)
    }

    pub(crate) fn toggle_actual(self, image: (u32, u32), frame: Size<f32>) -> Self {
        if (self.scale - 1.0).abs() < 1e-4 {
            Self {
                scale: 1.0 / pixel_ratio(image, frame),
                pan: Point::default(),
            }
            .clamped(image, frame)
        } else {
            Self::default()
        }
    }

    pub(crate) fn pan_by(self, delta: Point<f32>, image: (u32, u32), frame: Size<f32>) -> Self {
        Self {
            pan: self.pan + delta,
            ..self
        }
        .clamped(image, frame)
    }

    pub(crate) fn minimap_viewport(self, frame: Size<f32>, minimap: (f32, f32)) -> Bounds<f32> {
        let scale = self.scale.max(1.0);
        let width = (minimap.0 / scale).min(minimap.0);
        let height = (minimap.1 / scale).min(minimap.1);
        let center_x = 0.5 - self.pan.x / (frame.width * scale);
        let center_y = 0.5 - self.pan.y / (frame.height * scale);
        let left = (center_x * minimap.0 - width / 2.0).clamp(0.0, minimap.0 - width);
        let top = (center_y * minimap.1 - height / 2.0).clamp(0.0, minimap.1 - height);
        Bounds::new(Point { x: left, y: top }, Size { width, height })
    }

    pub(crate) fn pan_for_minimap(self, normalized: Point<f32>, frame: Size<f32>) -> Point<f32> {
        let scale = self.scale.max(1.0);
        let half = 0.5 / scale;
        let center_x = normalized.x.clamp(half, 1.0 - half);
        let center_y = normalized.y.clamp(half, 1.0 - half);
        point(
            -(center_x - 0.5) * frame.width * scale,
            -(center_y - 0.5) * frame.height * scale,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ViewerZoomDrag {
    Pan(Point<Pixels>),
    Minimap,
}

pub(crate) fn minimap_size(aspect: f32) -> (f32, f32) {
    let aspect = aspect.max(0.05);
    let width = VIEWER_MINIMAP_WIDTH.min(VIEWER_MINIMAP_HEIGHT * aspect);
    (width, width / aspect)
}

pub(crate) fn pixel_ratio(image: (u32, u32), frame: Size<f32>) -> f32 {
    if image.0 == 0 || image.1 == 0 || frame.width <= 0.0 || frame.height <= 0.0 {
        return 1.0;
    }
    (frame.width / image.0 as f32)
        .min(frame.height / image.1 as f32)
        .max(1e-4)
}

/// The smallest scale is fit or 100% actual pixels, whichever is smaller.
/// The largest is 400% actual pixels, but never below fit so tiny images keep
/// their fit-sized display.
fn zoom_limits(ratio: f32) -> (f32, f32) {
    let actual = 1.0 / ratio;
    (actual.min(1.0), (actual * 4.0).max(1.0))
}

/// A still image used by the viewer's crossfade: either a decoded full image
/// or a thumbnail layer borrowed from the texture array.
#[derive(Clone)]
pub(crate) enum ViewerStill {
    Full(Arc<FullImageTexture>),
    Thumbnail {
        id: i64,
        handle: ThumbnailHandle,
        dimensions: (u32, u32),
    },
}

#[derive(Clone)]
pub(crate) struct ViewerMorph {
    pub(crate) epoch: u64,
    pub(crate) started_at: Instant,
    pub(crate) from: Bounds<Pixels>,
    pub(crate) to: Bounds<Pixels>,
    pub(crate) thumbnail: Option<ViewerStill>,
    pub(crate) color: u32,
    pub(crate) source: ViewerOpenSource,
    pub(crate) closing: bool,
    pub(crate) fade_only: bool,
    pub(crate) backdrop_from: f32,
}

impl ViewerMorph {
    pub(crate) fn current_bounds(&self, now: Instant) -> Bounds<Pixels> {
        if self.fade_only {
            return self.from;
        }
        morph_bounds_at(self.from, self.to, self.started_at, now)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ViewerBoxState {
    pub(crate) epoch: u64,
    pub(crate) started_at: Instant,
    pub(crate) from_size: (f32, f32),
    pub(crate) to_aspect: f32,
    pub(crate) to_details_width: f32,
    pub(crate) duration: Duration,
}

impl ViewerBoxState {
    pub(crate) fn size_at(&self, t: f32, viewport: gpui_kit::Size<Pixels>) -> (f32, f32) {
        let t = t.clamp(0.0, 1.0);
        let to = viewer_image_box_with_details_width(
            viewport.width.into(),
            viewport.height.into(),
            self.to_details_width,
            self.to_aspect,
        );
        (
            self.from_size.0 + (to.0 - self.from_size.0) * t,
            self.from_size.1 + (to.1 - self.from_size.1) * t,
        )
    }

    pub(crate) fn current_size(&self, viewport: gpui_kit::Size<Pixels>) -> (f32, f32) {
        let t = eased_progress(self.started_at, self.duration, Instant::now());
        self.size_at(t, viewport)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ViewerPanelState {
    pub(crate) epoch: u64,
    pub(crate) started_at: Instant,
    pub(crate) from_width: f32,
    pub(crate) to_width: f32,
    pub(crate) duration: Duration,
}

impl ViewerPanelState {
    pub(crate) fn width_at(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        self.from_width + (self.to_width - self.from_width) * t
    }

    pub(crate) fn current_width(&self) -> f32 {
        self.current_width_at(Instant::now())
    }

    fn current_width_at(&self, now: Instant) -> f32 {
        self.width_at(eased_progress(self.started_at, self.duration, now))
    }
}

pub(crate) fn morph_bounds_at(
    from: Bounds<Pixels>,
    to: Bounds<Pixels>,
    started_at: Instant,
    now: Instant,
) -> Bounds<Pixels> {
    lerp_bounds(from, to, eased_progress(started_at, MOTION_MORPH, now))
}

pub(crate) fn current_tagged_bounds(
    entry: Option<(u64, Bounds<Pixels>)>,
    current_epoch: u64,
) -> Option<Bounds<Pixels>> {
    entry
        .filter(|(epoch, _)| *epoch == current_epoch)
        .map(|(_, bounds)| bounds)
}

/// Whether a navigation crossfade timer still owns the previous image. Only a
/// later navigation bumps the epoch and leaves the old timer stale; closing the
/// viewer cancels the release task and takes the image instead.
pub(crate) fn previous_image_release_due(epoch: u64, current_epoch: u64) -> bool {
    epoch == current_epoch
}

/// Delay before the outgoing crossfade image is released. Reduced motion
/// releases it immediately; otherwise it outlives the crossfade by a small
/// margin so the fade never loses its still.
pub(crate) fn previous_image_release_delay(reduce_motion: bool) -> Duration {
    if reduce_motion {
        Duration::ZERO
    } else {
        MOTION_BASE + Duration::from_millis(20)
    }
}

/// Navigation keys held down in the viewer. Key repeat is invisible to GPUI's
/// actions, so the key-up events are the stop signal.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ViewerNavKeys {
    left: bool,
    right: bool,
}

impl ViewerNavKeys {
    /// Marks one key as held.
    pub(crate) fn press(&mut self, left: bool) {
        if left {
            self.left = true;
        } else {
            self.right = true;
        }
    }

    /// Clears one key (`"left"` / `"right"`). Returns whether it was held.
    pub(crate) fn release(&mut self, key: &str) -> bool {
        match key {
            "left" => std::mem::take(&mut self.left),
            "right" => std::mem::take(&mut self.right),
            _ => false,
        }
    }

    pub(crate) fn any(self) -> bool {
        self.left || self.right
    }
}

/// A deferred full-image request may only start while the viewer is open and
/// the user is no longer stepping through photos.
pub(crate) fn should_request_full_image(open: bool, keys: ViewerNavKeys) -> bool {
    open && !keys.any()
}

pub(crate) fn preferred_bounds(
    prefer_card: bool,
    card: Option<Bounds<Pixels>>,
    inspector: Option<Bounds<Pixels>>,
) -> Option<Bounds<Pixels>> {
    if prefer_card {
        card.or(inspector)
    } else {
        inspector.or(card)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::{MOTION_BASE, MOTION_SLOW, VIEWER_DETAILS_WIDTH};
    use gpui_kit::{point, px, size};

    #[test]
    fn morph_bounds_at_hits_both_ends() {
        let start = Instant::now();
        let from = Bounds::new(point(px(0.), px(0.)), size(px(10.), px(10.)));
        let to = Bounds::new(point(px(100.), px(200.)), size(px(300.), px(400.)));
        assert_eq!(morph_bounds_at(from, to, start, start), from);
        assert_eq!(morph_bounds_at(from, to, start, start + MOTION_MORPH), to);
    }

    #[test]
    fn viewer_morph_current_bounds_follows_the_timeline() {
        let from = Bounds::new(point(px(10.), px(20.)), size(px(100.), px(80.)));
        let to = Bounds::new(point(px(200.), px(100.)), size(px(400.), px(300.)));
        let start = Instant::now();
        let morph = ViewerMorph {
            epoch: 1,
            started_at: start,
            from,
            to,
            thumbnail: None,
            color: 0,
            source: ViewerOpenSource::Card,
            closing: false,
            fade_only: false,
            backdrop_from: 1.0,
        };
        assert_eq!(morph.current_bounds(start), from);
        assert_eq!(morph.current_bounds(start + MOTION_MORPH), to);
        let middle = morph.current_bounds(start + MOTION_MORPH / 2);
        assert!(middle.origin.x > from.origin.x && middle.origin.x < to.origin.x);
    }

    #[test]
    fn fit_ratio_matches_the_displayed_pixels() {
        let image = (4000, 3000);
        let frame = size(800.0, 600.0);
        assert!((pixel_ratio(image, frame) - 0.2).abs() < 1e-6);
        assert_eq!(ViewerZoom::default().percent(image, frame), 20);
        assert_eq!(
            ViewerZoom {
                scale: 5.0,
                pan: Point::default(),
            }
            .percent(image, frame),
            100
        );
    }

    #[test]
    fn cursor_zoom_keeps_the_point_under_the_cursor() {
        let image = (4000, 3000);
        let frame = size(800.0, 600.0);
        let before = ViewerZoom {
            scale: 2.0,
            pan: point(40.0, -20.0),
        };
        let cursor = point(100.0, 50.0);
        let after = before.zoom_at(cursor, 1.25, image, frame);
        let point_before = (cursor - before.pan) / before.scale;
        let point_after = (cursor - after.pan) / after.scale;
        assert!((point_before.x - point_after.x).abs() < 0.01);
        assert!((point_before.y - point_after.y).abs() < 0.01);
    }

    #[test]
    fn zoom_clamps_at_fit_and_four_times_actual_pixels() {
        let image = (4000, 3000);
        let frame = size(800.0, 600.0);
        let fit = ViewerZoom::default();
        let max = fit.zoom_at(point(0.0, 0.0), 100.0, image, frame);
        assert_eq!(max.scale, 20.0);
        assert_eq!(max.percent(image, frame), 400);
        assert_eq!(fit.zoom_at(point(0.0, 0.0), 0.01, image, frame).scale, 1.0);
    }

    #[test]
    fn small_images_zoom_to_four_times_actual_pixels_without_losing_fit() {
        let image = (400, 300);
        let frame = size(800.0, 600.0);
        let fit = ViewerZoom::default();
        let max = fit.zoom_at(point(0.0, 0.0), 10.0, image, frame);
        assert_eq!(max.scale, 2.0);
        assert_eq!(max.percent(image, frame), 400);
        assert_eq!(fit.clamped(image, frame).scale, 1.0);
        assert_eq!(fit.clamped(image, frame).percent(image, frame), 200);
    }

    #[test]
    fn pan_clamps_at_the_edges_of_a_four_hundred_percent_zoom() {
        let image = (4000, 3000);
        let frame = size(800.0, 600.0);
        let zoomed = ViewerZoom::default()
            .zoom_at(point(0.0, 0.0), 100.0, image, frame)
            .pan_by(point(9999.0, -9999.0), image, frame);
        assert_eq!(zoomed.scale, 20.0);
        assert_eq!(zoomed.pan, point(7600.0, -5700.0));
    }

    #[test]
    fn double_click_reaches_actual_pixels_even_for_a_small_image() {
        let image = (400, 300);
        let frame = size(800.0, 600.0);
        let actual = ViewerZoom::default().toggle_actual(image, frame);
        assert_eq!(actual.scale, 0.5);
        assert_eq!(actual.percent(image, frame), 100);
        assert_eq!(actual.toggle_actual(image, frame), ViewerZoom::default());
    }

    #[test]
    fn pan_clamps_at_the_frame_and_centers_small_axes() {
        let image = (4000, 3000);
        let frame = size(800.0, 600.0);
        let zoomed = ViewerZoom {
            scale: 2.0,
            pan: point(9999.0, -9999.0),
        }
        .clamped(image, frame);
        assert_eq!(zoomed.pan, point(400.0, -300.0));
        let fit = ViewerZoom {
            scale: 1.0,
            pan: point(50.0, 50.0),
        }
        .clamped(image, frame);
        assert_eq!(fit.pan, Point::default());
    }

    #[test]
    fn a_frame_resize_reclamps_the_pan_around_the_center() {
        let image = (4000, 3000);
        let wide = size(800.0, 600.0);
        let narrow = size(500.0, 600.0);
        let zoomed = ViewerZoom {
            scale: 2.0,
            pan: point(400.0, 0.0),
        }
        .clamped(image, wide);
        let resized = zoomed.clamped(image, narrow);
        assert_eq!(resized.pan.x, 250.0);
        assert_eq!(resized.pan.y, 0.0);
    }

    #[test]
    fn minimap_size_keeps_the_image_aspect_inside_the_box() {
        let (width, height) = minimap_size(16.0 / 9.0);
        assert_eq!(width, 148.0);
        assert!((height - 83.25).abs() < 0.01);
        let (width, height) = minimap_size(1.0 / 2.0);
        assert_eq!((width.round(), height.round()), (55.0, 110.0));
    }

    #[test]
    fn minimap_viewport_tracks_zoom_and_pan() {
        let frame = size(800.0, 600.0);
        let minimap = (148.0, 111.0);
        let fit = ViewerZoom::default().minimap_viewport(frame, minimap);
        assert_eq!(fit.size.width, 148.0);
        assert_eq!(fit.origin.x, 0.0);

        let zoomed = ViewerZoom {
            scale: 2.0,
            pan: point(400.0, 0.0),
        }
        .clamped((4000, 3000), frame)
        .minimap_viewport(frame, minimap);
        assert!((zoomed.size.width - 74.0).abs() < 0.01);
        assert!((zoomed.origin.x - 0.0).abs() < 0.01);
    }

    #[test]
    fn minimap_click_centers_the_view_and_clamps_at_edges() {
        let frame = size(800.0, 600.0);
        let zoom = ViewerZoom {
            scale: 2.0,
            pan: Point::default(),
        };
        let centered = zoom.pan_for_minimap(point(0.5, 0.5), frame);
        assert_eq!(centered, Point::default());
        let right = zoom.pan_for_minimap(point(1.0, 1.0), frame);
        assert_eq!(right, point(-400.0, -300.0));
    }

    #[test]
    fn viewer_box_state_interpolates_from_the_visible_size() {
        let viewport = size(px(1200.), px(800.));
        let from_size = (800.0, 500.0);
        let state = ViewerBoxState {
            epoch: 1,
            started_at: Instant::now(),
            from_size,
            to_aspect: 2.0,
            to_details_width: 0.0,
            duration: MOTION_BASE,
        };
        let (w0, h0) = state.size_at(0.0, viewport);
        let (w1, h1) = state.size_at(1.0, viewport);
        let (wm, hm) = state.size_at(0.5, viewport);
        assert_eq!((w0, h0), from_size);
        assert_eq!(w1, 1152.0);
        assert_eq!(h1, 576.0);
        assert!(wm > w0 && wm < w1);
        assert!(hm > h0 && hm < h1);
    }

    #[test]
    fn viewer_box_state_hits_both_details_ends() {
        let viewport = size(px(1200.), px(800.));
        let state = ViewerBoxState {
            epoch: 2,
            started_at: Instant::now(),
            from_size: (900.0, 560.0),
            to_aspect: 16.0 / 10.0,
            to_details_width: VIEWER_DETAILS_WIDTH,
            duration: MOTION_SLOW,
        };
        let (closed, _) = state.size_at(0.0, viewport);
        let (open, _) = state.size_at(1.0, viewport);
        assert!(closed > open);
    }

    #[test]
    fn viewer_panel_state_retargets_from_the_current_width() {
        let start = Instant::now();
        let state = ViewerPanelState {
            epoch: 1,
            started_at: start,
            from_width: 120.0,
            to_width: 0.0,
            duration: MOTION_SLOW,
        };
        assert_eq!(state.width_at(0.0), 120.0);
        assert_eq!(state.width_at(1.0), 0.0);
        let middle = state.width_at(0.5);
        assert!(middle > 0.0 && middle < 120.0);
        assert_eq!(state.current_width_at(start + MOTION_SLOW), 0.0);
    }

    #[test]
    fn morph_radius_plan_covers_card_and_inspector_paths() {
        assert_eq!(
            morph_radius_plan(false, ViewerOpenSource::Card),
            (12.0, 10.0)
        );
        assert_eq!(
            morph_radius_plan(false, ViewerOpenSource::Inspector),
            (11.0, 10.0)
        );
        assert_eq!(
            morph_radius_plan(true, ViewerOpenSource::Card),
            (10.0, 12.0)
        );
        assert_eq!(
            morph_radius_plan(true, ViewerOpenSource::Inspector),
            (10.0, 11.0)
        );
    }

    #[test]
    fn current_tagged_bounds_rejects_stale_frames() {
        let bounds = Bounds::new(point(px(1.), px(2.)), size(px(3.), px(4.)));
        assert_eq!(current_tagged_bounds(Some((7, bounds)), 7), Some(bounds));
        assert_eq!(current_tagged_bounds(Some((7, bounds)), 8), None);
        assert_eq!(current_tagged_bounds(None, 7), None);
    }

    #[test]
    fn previous_image_release_only_matches_the_current_epoch() {
        assert!(previous_image_release_due(7, 7));
        assert!(!previous_image_release_due(7, 8));
    }

    #[test]
    fn previous_image_release_delay_follows_reduced_motion() {
        assert_eq!(previous_image_release_delay(true), Duration::ZERO);
        assert_eq!(
            previous_image_release_delay(false),
            Duration::from_millis(240)
        );
    }

    #[test]
    fn preferred_bounds_falls_back_to_the_other_source() {
        let card = Bounds::new(point(px(0.), px(0.)), size(px(10.), px(10.)));
        let inspector = Bounds::new(point(px(20.), px(20.)), size(px(30.), px(30.)));
        assert_eq!(
            preferred_bounds(true, Some(card), Some(inspector)),
            Some(card)
        );
        assert_eq!(
            preferred_bounds(true, None, Some(inspector)),
            Some(inspector)
        );
        assert_eq!(
            preferred_bounds(false, Some(card), Some(inspector)),
            Some(inspector)
        );
        assert_eq!(preferred_bounds(false, Some(card), None), Some(card));
    }

    #[test]
    fn closing_prefers_the_original_source() {
        let card = Bounds::new(point(px(0.), px(0.)), size(px(10.), px(10.)));
        let inspector = Bounds::new(point(px(20.), px(20.)), size(px(30.), px(30.)));
        assert_eq!(
            preferred_bounds(true, Some(card), Some(inspector)),
            Some(card)
        );
        assert_eq!(
            preferred_bounds(false, Some(card), Some(inspector)),
            Some(inspector)
        );
    }

    #[test]
    fn nav_keys_gate_full_image_requests() {
        let mut left = ViewerNavKeys::default();
        left.press(true);
        let mut right = ViewerNavKeys::default();
        right.press(false);
        let mut both = left;
        both.press(false);

        assert!(should_request_full_image(true, ViewerNavKeys::default()));
        assert!(!should_request_full_image(false, ViewerNavKeys::default()));
        assert!(!should_request_full_image(true, left));
        assert!(!should_request_full_image(true, right));
        assert!(!should_request_full_image(true, both));
    }

    #[test]
    fn releasing_one_nav_key_keeps_the_other_held() {
        let mut keys = ViewerNavKeys::default();
        keys.press(true);
        keys.press(false);

        assert!(keys.release("left"));
        assert!(keys.any(), "the other key is still down");
        assert!(keys.release("right"));
        assert!(!keys.any());
        assert!(
            !keys.release("right"),
            "an unheld key is not released twice"
        );
        assert!(!keys.release("escape"), "unrelated keys are ignored");
    }
}
