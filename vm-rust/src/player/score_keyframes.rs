use std::collections::HashMap;

use crate::director::chunks::score::{ScoreFrameChannelData, FrameIntervalPrimary, TweenInfo};
use crate::player::sprite::ColorRef;

pub trait DirectorProperty: Clone + PartialEq {
    type Raw;

    const USE_BASELINE_SKIP: bool = false;

    /// Extract raw value (if present) from frame data
    fn extract_raw(data: &ScoreFrameChannelData) -> Option<Self::Raw>;

    /// Convert raw value into a resolved property value
    fn resolve_raw(raw: Self::Raw) -> Self;  
    
    /// Default value when property never appears
    fn default() -> Self;

    /// resolve with previous value (delta-aware)
    fn resolve_with_prev(raw: Self::Raw, prev: Option<&Self>) -> Self {
        Self::resolve_raw(raw)
    }
    
    /// Check if this value is a standard default (e.g., PaletteIndex 0 or 255 for colors)
    /// Used to filter out false positive "animations" between default values
    fn is_standard_default(&self) -> bool {
        false
    }
}

/// Resolves effective values per frame and returns only real changes
/// Filters out frames with default values
pub fn collect_property_keyframes<P: DirectorProperty>(
    frames: &[(u32, u16, ScoreFrameChannelData)],
) -> Vec<(u32, P)> {
    let mut keyframes = Vec::new();
    let default_val = P::default();
    let mut current = P::default();
    let mut initialized = false;

    // For colors: get the sprite's baseline color from first frame
    let baseline = if P::USE_BASELINE_SKIP && !frames.is_empty() {
        if let Some(raw) = P::extract_raw(&frames[0].2) {
            Some(P::resolve_raw(raw))
        } else {
            None
        }
    } else {
        None
    };

    // For colors: check if there's any value different from baseline
    // If so, we need to include the baseline as the first keyframe
    let has_animation = if P::USE_BASELINE_SKIP {
        if let Some(ref baseline_val) = baseline {
            frames.iter().any(|(_, _, data)| {
                if let Some(raw) = P::extract_raw(data) {
                    P::resolve_raw(raw) != *baseline_val
                } else {
                    false
                }
            })
        } else {
            false
        }
    } else {
        false
    };

    let mut last_value: Option<P> = None;

    for (frame_num, _, data) in frames {
        if let Some(raw) = P::extract_raw(data) {
            let resolved = P::resolve_with_prev(raw, last_value.as_ref());

            // Update last_value BEFORE any continue statements
            // so resolve_with_prev has access to previous value on next iteration
            last_value = Some(resolved.clone());

            // Skip if this is a default value
            if P::USE_BASELINE_SKIP {
                // For colors: if there's animation, include baseline as first keyframe
                // then skip subsequent baseline values but include changed values
                if let Some(ref baseline_val) = baseline {
                    if resolved == *baseline_val {
                        // Only include baseline if it's the first frame AND there's animation
                        if has_animation && !initialized {
                            keyframes.push((*frame_num, resolved.clone()));
                            current = resolved;
                            initialized = true;
                        }
                        continue;
                    }
                }
            } else {
                // normal logic
                if resolved == default_val {
                    continue;
                }
            }

            if !initialized || resolved != current {
                keyframes.push((*frame_num, resolved.clone()));
                current = resolved.clone();
                initialized = true;
            }
        }
    }

    keyframes
}

/// Checks if property actually animates across the interval
/// Only considers non-default values
pub fn has_real_animation<P: DirectorProperty + std::fmt::Debug>(
    frames: &[(u32, u16, ScoreFrameChannelData)],
) -> bool {
    let default_val = P::default();
    let mut values: Vec<P> = Vec::new();

    if P::USE_BASELINE_SKIP {
        // For colors: animation exists if ANY frame has a non-standard-default color
        // Standard defaults are PaletteIndex(0) and PaletteIndex(255)
        // If all frames only have 0 or 255, it's just sprite initialization, not animation
        // But if any frame has a different color (like 14), that's real animation
        
        for (_, _, data) in frames {
            if let Some(raw) = P::extract_raw(data) {
                let current = P::resolve_raw(raw);
                
                if !current.is_standard_default() {
                    // Found a non-standard-default color - that's real animation!
                    return true;
                }
            }
        }
        
        // All frames only have standard defaults (0 or 255) - no animation
        return false;
    }

    // Non-color properties: original logic
    for (_, _, data) in frames {
        if let Some(raw) = P::extract_raw(data) {
            let current = P::resolve_raw(raw);
            
            // Only consider non-default values
            if current != default_val {
                values.push(current);
            }
        }
    }

    // Need at least 2 different non-default values to be real animation
    if values.len() < 2 {
        return false;
    }

    let first = &values[0];
    values.iter().any(|v| v != first)
}

/// Combined position property (for path keyframes)
#[derive(Clone, PartialEq, Debug)]
pub struct Position(pub i16, pub i16);

impl DirectorProperty for Position {
    const USE_BASELINE_SKIP: bool = false;
    type Raw = (i16, i16);

    fn extract_raw(data: &ScoreFrameChannelData) -> Option<Self::Raw> {
        Some((data.pos_x, data.pos_y))
    }

    fn resolve_raw(raw: Self::Raw) -> Self {
        Position(raw.0, raw.1)
    }

    fn resolve_with_prev(raw: Self::Raw, prev: Option<&Self>) -> Self {
        match prev {
            None => Position(raw.0, raw.1),
            Some(p) => Position(
                if raw.0 == 0 { p.0 } else { raw.0 },
                if raw.1 == 0 { p.1 } else { raw.1 },
            ),
        }
    }

    fn default() -> Self {
        Position(0, 0)
    }
}

/// Combined size property (for size keyframes)
#[derive(Clone, PartialEq, Debug)]
pub struct Size(pub i32, pub i32);

impl DirectorProperty for Size {
    const USE_BASELINE_SKIP: bool = false;
    type Raw = (i32, i32);

    fn extract_raw(data: &ScoreFrameChannelData) -> Option<Self::Raw> {
        Some((data.width as i32, data.height as i32))
    }

    fn resolve_raw(raw: Self::Raw) -> Self {
        Size(raw.0, raw.1)
    }

    fn resolve_with_prev(raw: Self::Raw, prev: Option<&Self>) -> Self {
        match prev {
            None => Size(raw.0, raw.1),
            Some(p) => Size(
                if raw.0 == 0 { p.0 } else { raw.0 },
                if raw.1 == 0 { p.1 } else { raw.1 },
            ),
        }
    }

    fn default() -> Self {
        Size(0, 0)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct ForeColor(pub ColorRef);

impl DirectorProperty for ForeColor {
    const USE_BASELINE_SKIP: bool = true;
    type Raw = ColorRef;

    fn extract_raw(data: &ScoreFrameChannelData) -> Option<Self::Raw> {
        let color_ref = match data.color_flag {
            // fore is palette index
            0 | 2 => ColorRef::PaletteIndex(data.fore_color),
            
            // fore is RGB
            1 | 3 => ColorRef::Rgb(
                data.fore_color,
                data.fore_color_g,
                data.fore_color_b,
            ),
            
            _ => ColorRef::PaletteIndex(data.fore_color),
        };
        
        Some(color_ref)
    }

    fn resolve_raw(raw: Self::Raw) -> Self {
        ForeColor(raw)
    }

    fn resolve_with_prev(raw: Self::Raw, prev: Option<&Self>) -> Self {
        // If color is PaletteIndex(0), keep the previous value
        match (&raw, prev) {
            (ColorRef::PaletteIndex(0), Some(p)) => p.clone(),
            _ => ForeColor(raw),
        }
    }

    fn default() -> Self {
        ForeColor(ColorRef::PaletteIndex(255))
    }
    
    fn is_standard_default(&self) -> bool {
        match &self.0 {
            ColorRef::PaletteIndex(0) | ColorRef::PaletteIndex(255) => true,
            _ => false,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct BackColor(pub ColorRef);

impl DirectorProperty for BackColor {
    const USE_BASELINE_SKIP: bool = true;
    type Raw = ColorRef;

    fn extract_raw(data: &ScoreFrameChannelData) -> Option<Self::Raw> {
        let color_ref = match data.color_flag {
            // back is palette index
            0 | 1 => ColorRef::PaletteIndex(data.back_color),
            
            // back is RGB
            2 | 3 => ColorRef::Rgb(
                data.back_color,
                data.back_color_g,
                data.back_color_b,
            ),
            
            _ => ColorRef::PaletteIndex(data.back_color),
        };
        
        Some(color_ref)
    }

    fn resolve_raw(raw: Self::Raw) -> Self {
        BackColor(raw)
    }

    fn resolve_with_prev(raw: Self::Raw, prev: Option<&Self>) -> Self {
        // If color is PaletteIndex(0), keep the previous value
        match (&raw, prev) {
            (ColorRef::PaletteIndex(0), Some(p)) => p.clone(),
            _ => BackColor(raw),
        }
    }

    fn default() -> Self {
        BackColor(ColorRef::PaletteIndex(0))
    }
    
    fn is_standard_default(&self) -> bool {
        match &self.0 {
            ColorRef::PaletteIndex(0) | ColorRef::PaletteIndex(255) => true,
            _ => false,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Rotation(pub f64);

impl DirectorProperty for Rotation {
    const USE_BASELINE_SKIP: bool = false;
    type Raw = f64;

    fn extract_raw(data: &ScoreFrameChannelData) -> Option<Self::Raw> {
        Some(data.rotation)
    }

    fn resolve_raw(raw: Self::Raw) -> Self {
        Rotation(raw)
    }

    fn default() -> Self {
        Rotation(0.0)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Skew(pub f64);

impl DirectorProperty for Skew {
    const USE_BASELINE_SKIP: bool = false;
    type Raw = f64;

    fn extract_raw(data: &ScoreFrameChannelData) -> Option<Self::Raw> {
        Some(data.skew)
    }

    fn resolve_raw(raw: Self::Raw) -> Self {
        Skew(raw)
    }

    fn default() -> Self {
        Skew(0.0)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Blend(pub i32);

impl DirectorProperty for Blend {
    const USE_BASELINE_SKIP: bool = false;
    type Raw = i32;

    fn extract_raw(data: &ScoreFrameChannelData) -> Option<Self::Raw> {
        Some(data.blend as i32)
    }

    fn resolve_raw(raw: Self::Raw) -> Self {
        Blend(raw)
    }

    fn default() -> Self {
        Blend(100)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct LocH(pub i16);

impl DirectorProperty for LocH {
    const USE_BASELINE_SKIP: bool = false;
    type Raw = i16;

    fn extract_raw(data: &ScoreFrameChannelData) -> Option<Self::Raw> {
        Some(data.pos_x)
    }

    fn resolve_raw(raw: Self::Raw) -> Self {
        LocH(raw)
    }

    fn default() -> Self {
        LocH(0)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct LocV(pub i16);

impl DirectorProperty for LocV {
    const USE_BASELINE_SKIP: bool = false;
    type Raw = i16;

    fn extract_raw(data: &ScoreFrameChannelData) -> Option<Self::Raw> {
        Some(data.pos_y)
    }

    fn resolve_raw(raw: Self::Raw) -> Self {
        LocV(raw)
    }

    fn default() -> Self {
        LocV(0)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Width(pub i32);

impl DirectorProperty for Width {
    const USE_BASELINE_SKIP: bool = false;
    type Raw = i32;

    fn extract_raw(data: &ScoreFrameChannelData) -> Option<Self::Raw> {
        Some(data.width as i32)
    }

    fn resolve_raw(raw: Self::Raw) -> Self {
        Width(raw)
    }

    fn default() -> Self {
        Width(0)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Height(pub i32);

impl DirectorProperty for Height {
    const USE_BASELINE_SKIP: bool = false;
    type Raw = i32;

    fn extract_raw(data: &ScoreFrameChannelData) -> Option<Self::Raw> {
        Some(data.height as i32)
    }

    fn resolve_raw(raw: Self::Raw) -> Self {
        Height(raw)
    }

    fn default() -> Self {
        Height(0)
    }
}


/// Convert channel index (stored in score) to channel number (displayed in Director)
pub fn index_to_channel_number(index: u16) -> u16 {
    if index <= 5 {
        index
    } else {
        index - 5
    }
}

/// Curvature type for path interpolation
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CurvatureType {
    Linear = 0,   // Straight line
    Normal = 1,   // Curved path inside keyframes
    Extreme = 2,  // Curved path outside keyframes
}

impl From<u32> for CurvatureType {
    fn from(val: u32) -> Self {
        // Curvature comes as 65536 (0x10000) for value 1.0 in fixed-point
        // Convert from fixed-point to integer: divide by 65536
        let normalized = (val / 65536).min(2);
        match normalized {
            0 => CurvatureType::Linear,
            1 => CurvatureType::Normal,
            2 => CurvatureType::Extreme,
            _ => CurvatureType::Normal,
        }
    }
}

// Helper function (should be at module level in score_keyframes.rs)
fn apply_easing(t: f64, ease_in: u32, ease_out: u32, smooth_speed: bool) -> f64 {
    if !smooth_speed {
        return t;
    }
    
    let ease_in_pct = ease_in as f64 / 100.0;
    let ease_out_pct = ease_out as f64 / 100.0;
    
    let total_ease = ease_in_pct + ease_out_pct;
    let (ease_in_norm, ease_out_norm) = if total_ease > 1.0 {
        (ease_in_pct / total_ease, ease_out_pct / total_ease)
    } else {
        (ease_in_pct, ease_out_pct)
    };
    
    if t < ease_in_norm {
        let local_t = t / ease_in_norm;
        ease_in_norm * local_t * local_t
    } else if t > (1.0 - ease_out_norm) {
        let local_t = (t - (1.0 - ease_out_norm)) / ease_out_norm;
        1.0 - ease_out_norm + ease_out_norm * (1.0 - (1.0 - local_t) * (1.0 - local_t))
    } else {
        t
    }
}

/// Apply curvature to interpolation
fn apply_curvature(t: f64, curvature: CurvatureType) -> f64 {
    match curvature {
        CurvatureType::Linear => t,
        CurvatureType::Normal => {
            // Smooth S-curve using cubic ease-in-out
            if t < 0.5 {
                4.0 * t * t * t
            } else {
                1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
            }
        }
        CurvatureType::Extreme => {
            // More extreme S-curve
            let t2 = t * t;
            let t3 = t2 * t;
            3.0 * t2 - 2.0 * t3
        }
    }
}

/// Represents a single blend keyframe
#[derive(Clone, Debug)]
pub struct BlendKeyframe {
    pub frame: u32,
    pub blend_percent: u8,
}

/// Represents a single rotation keyframe
#[derive(Clone, Debug)]
pub struct RotationKeyframe {
    pub frame: u32,
    pub rotation: f64,
}

/// Represents a single skew keyframe
#[derive(Clone, Debug)]
pub struct SkewKeyframe {
    pub frame: u32,
    pub skew: f64,
}

/// Tracks blend keyframes for a single sprite channel
#[derive(Clone, Debug)]
pub struct SpriteBlendKeyframes {
    pub channel: u16,
    pub keyframes: Vec<BlendKeyframe>,
    pub tween_info: Option<TweenInfo>,
    pub intervals: Vec<(u32, u32)>,
}

/// Tracks rotation keyframes for a single sprite channel
#[derive(Clone, Debug)]
pub struct SpriteRotationKeyframes {
    pub channel: u16,
    pub keyframes: Vec<RotationKeyframe>,
    pub tween_info: Option<TweenInfo>,
    pub intervals: Vec<(u32, u32)>,
}

/// Represents a single skew keyframe
#[derive(Clone, Debug)]
pub struct SpriteSkewKeyframes {
    pub channel: u16,
    pub keyframes: Vec<SkewKeyframe>,
    pub tween_info: Option<TweenInfo>,
    pub intervals: Vec<(u32, u32)>,
}

/// Represents a single path (position) keyframe
#[derive(Clone, Debug)]
pub struct PathKeyframe {
    pub frame: u32,
    pub x: i16,
    pub y: i16,
}

/// Tracks path keyframes for a single sprite channel
#[derive(Clone, Debug)]
pub struct SpritePathKeyframes {
    pub channel: u16,
    pub keyframes: Vec<PathKeyframe>,
    pub tween_info: Option<TweenInfo>,
    pub intervals: Vec<(u32, u32)>,
}

/// Represents a single size keyframe
#[derive(Clone, Debug)]
pub struct SizeKeyframe {
    pub frame: u32,
    pub width: u16,
    pub height: u16,
}

/// Tracks size keyframes for a single sprite channel
#[derive(Clone, Debug)]
pub struct SpriteSizeKeyframes {
    pub channel: u16,
    pub keyframes: Vec<SizeKeyframe>,
    pub tween_info: Option<TweenInfo>,
    pub intervals: Vec<(u32, u32)>,
}

/// Represents a single color keyframe
#[derive(Clone, Debug)]
pub struct ColorKeyframe {
    pub frame: u32,
    pub color: ColorRef,
}

/// Tracks foreground color keyframes for a single sprite channel
#[derive(Clone, Debug)]
pub struct SpriteForeColorKeyframes {
    pub channel: u16,
    pub keyframes: Vec<ColorKeyframe>,
    pub tween_info: Option<TweenInfo>,
    pub intervals: Vec<(u32, u32)>,
}

/// Tracks background color keyframes for a single sprite channel
#[derive(Clone, Debug)]
pub struct SpriteBackColorKeyframes {
    pub channel: u16,
    pub keyframes: Vec<ColorKeyframe>,
    pub tween_info: Option<TweenInfo>,
    pub intervals: Vec<(u32, u32)>,
}


pub trait KeyframeTrack {
    // Add these new methods to access the common fields
    type Keyframe: KeyframeData;
    fn get_keyframes(&self) -> &[Self::Keyframe];
    fn get_intervals(&self) -> &[(u32, u32)];
    
    fn frame_range(&self) -> Option<(u32, u32)>;

    fn is_active_at_frame(&self, frame: u32) -> bool {
        let keyframes = self.get_keyframes();
        let intervals = self.get_intervals();
        
        if keyframes.is_empty() {
            return false;
        }
        
        // Case 1: Frame is exactly on a keyframe - always active
        if keyframes.iter().any(|kf| kf.frame() == frame) {
            return true;
        }
        
        // Case 2: Frame is between two keyframes - check if they're in same interval
        let prev_kf = keyframes.iter()
            .rev()
            .find(|kf| kf.frame() <= frame);
        
        let next_kf = keyframes.iter()
            .find(|kf| kf.frame() > frame);
        
        if let (Some(prev), Some(next)) = (prev_kf, next_kf) {
            let prev_frame = prev.frame();
            let next_frame = next.frame();
            
            // Check if both keyframes and current frame are in the same interval
            for (start, end) in intervals {
                if prev_frame >= *start && prev_frame <= *end &&
                   next_frame >= *start && next_frame <= *end &&
                   frame >= *start && frame <= *end {
                    return true;
                }
            }
        }
        
        false
    }
}

pub trait KeyframeData {
    fn frame(&self) -> u32;
}

impl KeyframeData for BlendKeyframe {
    fn frame(&self) -> u32 { self.frame }
}

impl KeyframeData for RotationKeyframe {
    fn frame(&self) -> u32 { self.frame }
}

impl KeyframeData for SkewKeyframe {
    fn frame(&self) -> u32 { self.frame }
}

impl KeyframeData for PathKeyframe {
    fn frame(&self) -> u32 { self.frame }
}

impl KeyframeData for SizeKeyframe {
    fn frame(&self) -> u32 { self.frame }
}

impl KeyframeData for ColorKeyframe {
    fn frame(&self) -> u32 { self.frame }
}

impl KeyframeTrack for SpriteBlendKeyframes {
    type Keyframe = BlendKeyframe;
    
    fn get_keyframes(&self) -> &[Self::Keyframe] {
        &self.keyframes
    }
    
    fn get_intervals(&self) -> &[(u32, u32)] {
        &self.intervals
    }
    
    fn frame_range(&self) -> Option<(u32, u32)> {
        let first = self.keyframes.first()?.frame;
        let last  = self.keyframes.last()?.frame;
        Some((first, last))
    }
}

impl KeyframeTrack for SpriteRotationKeyframes {
    type Keyframe = RotationKeyframe;
    
    fn get_keyframes(&self) -> &[Self::Keyframe] {
        &self.keyframes
    }
    
    fn get_intervals(&self) -> &[(u32, u32)] {
        &self.intervals
    }
    
    fn frame_range(&self) -> Option<(u32, u32)> {
        let first = self.keyframes.first()?.frame;
        let last  = self.keyframes.last()?.frame;
        Some((first, last))
    }
}

impl KeyframeTrack for SpriteSkewKeyframes {
    type Keyframe = SkewKeyframe;
    
    fn get_keyframes(&self) -> &[Self::Keyframe] {
        &self.keyframes
    }
    
    fn get_intervals(&self) -> &[(u32, u32)] {
        &self.intervals
    }
    
    fn frame_range(&self) -> Option<(u32, u32)> {
        let first = self.keyframes.first()?.frame;
        let last  = self.keyframes.last()?.frame;
        Some((first, last))
    }
}

impl KeyframeTrack for SpritePathKeyframes {
    type Keyframe = PathKeyframe;
    
    fn get_keyframes(&self) -> &[Self::Keyframe] {
        &self.keyframes
    }
    
    fn get_intervals(&self) -> &[(u32, u32)] {
        &self.intervals
    }
    
    fn frame_range(&self) -> Option<(u32, u32)> {
        let first = self.keyframes.first()?.frame;
        let last  = self.keyframes.last()?.frame;
        Some((first, last))
    }
}

impl KeyframeTrack for SpriteSizeKeyframes {
    type Keyframe = SizeKeyframe;
    
    fn get_keyframes(&self) -> &[Self::Keyframe] {
        &self.keyframes
    }
    
    fn get_intervals(&self) -> &[(u32, u32)] {
        &self.intervals
    }
    
    fn frame_range(&self) -> Option<(u32, u32)> {
        let first = self.keyframes.first()?.frame;
        let last  = self.keyframes.last()?.frame;
        Some((first, last))
    }
}

impl KeyframeTrack for SpriteForeColorKeyframes {
    type Keyframe = ColorKeyframe;
    
    fn get_keyframes(&self) -> &[Self::Keyframe] {
        &self.keyframes
    }
    
    fn get_intervals(&self) -> &[(u32, u32)] {
        &self.intervals
    }
    
    fn frame_range(&self) -> Option<(u32, u32)> {
        let first = self.keyframes.first()?.frame;
        let last  = self.keyframes.last()?.frame;
        Some((first, last))
    }
}

impl KeyframeTrack for SpriteBackColorKeyframes {
    type Keyframe = ColorKeyframe;
    
    fn get_keyframes(&self) -> &[Self::Keyframe] {
        &self.keyframes
    }
    
    fn get_intervals(&self) -> &[(u32, u32)] {
        &self.intervals
    }
    
    fn frame_range(&self) -> Option<(u32, u32)> {
        let first = self.keyframes.first()?.frame;
        let last  = self.keyframes.last()?.frame;
        Some((first, last))
    }
}

impl SpriteBlendKeyframes {
    pub fn new(channel_index: u16, tween_info: Option<TweenInfo>) -> Self {
        Self {
            channel: index_to_channel_number(channel_index),
            keyframes: Vec::new(),
            tween_info,
            intervals: Vec::new(),
        }
    }

    pub fn from_frame_data(
        frame_channel_data: &Vec<(u32, u16, ScoreFrameChannelData)>,
        channel_index: u16,
        intervals: Option<&Vec<FrameIntervalPrimary>>,
    ) -> Self {
        let Some(intervals) = intervals else {
            return Self::new(channel_index, None);
        };

        // Use first interval's tween_info for the struct (they should all be consistent)
        let tween_info = intervals.first().map(|i| i.tween_info.clone());
        let mut keyframes = Self::new(channel_index, tween_info);

        // Store the interval ranges
        keyframes.intervals = intervals.iter()
            .map(|i| (i.start_frame, i.end_frame))
            .collect();

        // Process each interval
        for interval in intervals {
            // Ignore single-frame intervals
            if interval.start_frame == interval.end_frame {
                continue;
            }

            // Blend tween not enabled in this interval
            if !interval.tween_info.is_blend_tweened() {
                continue;
            }

            let start_frame = interval.start_frame;
            let end_frame = interval.end_frame;

            // 1️⃣ Collect frames in Director frame range AND matching this channel
            let frames: Vec<_> = frame_channel_data
                .iter()
                .filter(|(frame_num, ch, _)| {
                    let director_frame = *frame_num + 1;
                    let in_range = director_frame >= start_frame && director_frame <= end_frame;
                    let matches_channel = *ch == channel_index;
                    in_range && matches_channel
                })
                .collect();

            if frames.is_empty() {
                continue;
            }

            // 2️⃣ Deduplicate by frame number (shouldn't be needed now, but keep for safety)
            let mut dedup: HashMap<u32, &(u32, u16, ScoreFrameChannelData)> = HashMap::new();

            for frame in frames {
                let (frame_num, _, _) = frame;
                dedup.insert(*frame_num, frame);
            }

            let mut sorted_frames: Vec<_> = dedup.into_iter().map(|(_, v)| v.clone()).collect();
            sorted_frames.sort_by_key(|(frame_num, _, _)| *frame_num);

            // 3️⃣ Check if blend ACTUALLY animates in this interval
            if !has_real_animation::<Blend>(&sorted_frames) {
                continue;
            }

            // 4️⃣ Collect effective keyframes for this interval
            let property_frames = collect_property_keyframes::<Blend>(&sorted_frames);

            for (frame, Blend(blend_val)) in property_frames {
                let blend_percent = convert_blend_to_percentage(blend_val as u8);
                keyframes.keyframes.push(BlendKeyframe {
                    frame,
                    blend_percent,
                });
            }
        }

        // Sort all keyframes by frame number after collecting from all intervals
        keyframes.keyframes.sort_by_key(|kf| kf.frame);

        // 5️⃣ Debug output
        if !keyframes.keyframes.is_empty() {
            web_sys::console::log_1(&format!(
                "🎨 Channel {} has {} blend keyframes across {} intervals",
                keyframes.channel,
                keyframes.keyframes.len(),
                intervals.len()
            ).into());

            for kf in &keyframes.keyframes {
                web_sys::console::log_1(
                    &format!("   Frame {}: {}%", kf.frame, kf.blend_percent).into(),
                );
            }
        }

        keyframes
    }

    pub fn get_blend_at_frame(&self, frame: u32) -> Option<u8> {
        if self.keyframes.is_empty() {
            return None;
        }

        // Check if frame is within the active range
        if !self.is_active_at_frame(frame) {
            return None;
        }

        let current_kf = self.keyframes
            .iter()
            .rev()
            .find(|kf| kf.frame <= frame);

        current_kf.map(|kf| kf.blend_percent)
    }

    pub fn get_value_at_frame(&self, frame: u32) -> Option<u8> {
        self.get_blend_at_frame(frame)
    }
}

impl SpriteRotationKeyframes {
    pub fn new(channel_index: u16, tween_info: Option<TweenInfo>) -> Self {
        Self {
            channel: index_to_channel_number(channel_index),
            keyframes: Vec::new(),
            tween_info,
            intervals: Vec::new(),
        }
    }

    pub fn from_frame_data(
        frame_channel_data: &Vec<(u32, u16, ScoreFrameChannelData)>,
        channel_index: u16,
        intervals: Option<&Vec<FrameIntervalPrimary>>,
    ) -> Self {
        let Some(intervals) = intervals else {
            return Self::new(channel_index, None);
        };

        // Use first interval's tween_info for the struct (they should all be consistent)
        let tween_info = intervals.first().map(|i| i.tween_info.clone());
        let mut keyframes = Self::new(channel_index, tween_info);

        // Store the interval ranges
        keyframes.intervals = intervals.iter()
            .map(|i| (i.start_frame, i.end_frame))
            .collect();

        // Process each interval
        for interval in intervals {
            // Ignore single-frame intervals
            if interval.start_frame == interval.end_frame {
                continue;
            }

            // Rotation tween not enabled in this interval
            if !interval.tween_info.is_rotation_tweened() {
                continue;
            }

            let start_frame = interval.start_frame;
            let end_frame = interval.end_frame;

            // 1️⃣ Collect frames in Director frame range AND matching this channel
            let frames: Vec<_> = frame_channel_data
                .iter()
                .filter(|(frame_num, ch, _)| {
                    let director_frame = *frame_num + 1;
                    let in_range = director_frame >= start_frame && director_frame <= end_frame;
                    let matches_channel = *ch == channel_index;
                    in_range && matches_channel
                })
                .collect();

            if frames.is_empty() {
                continue;
            }

            // 2️⃣ Deduplicate by frame number (shouldn't be needed now, but keep for safety)
            let mut dedup: HashMap<u32, &(u32, u16, ScoreFrameChannelData)> = HashMap::new();

            for frame in frames {
                let (frame_num, _, _) = frame;
                dedup.insert(*frame_num, frame);
            }

            let mut sorted_frames: Vec<_> = dedup.into_iter().map(|(_, v)| v.clone()).collect();
            sorted_frames.sort_by_key(|(frame_num, _, _)| *frame_num);

            // 3️⃣ Check if rotation ACTUALLY animates in this interval
            if !has_real_animation::<Rotation>(&sorted_frames) {
                continue;
            }

            // 4️⃣ Collect effective keyframes for this interval
            let property_frames = collect_property_keyframes::<Rotation>(&sorted_frames);

            for (frame, Rotation(rotation)) in property_frames {
                keyframes.keyframes.push(RotationKeyframe {
                    frame,
                    rotation,
                });
            }
        }

        // Sort all keyframes by frame number after collecting from all intervals
        keyframes.keyframes.sort_by_key(|kf| kf.frame);

        // 5️⃣ Debug output
        if !keyframes.keyframes.is_empty() {
            web_sys::console::log_1(&format!(
                "🔄 Channel {} has {} rotation keyframes across {} intervals",
                keyframes.channel,
                keyframes.keyframes.len(),
                intervals.len()
            ).into());
            
            for kf in &keyframes.keyframes {
                web_sys::console::log_1(&format!(
                    "   Frame {}: {:.2}°",
                    kf.frame,
                    kf.rotation
                ).into());
            }
        }

        keyframes
    }

    pub fn get_rotation_at_frame(&self, frame: u32) -> Option<f64> {
        if self.keyframes.is_empty() {
            return None;
        }

        // Check if frame is within the active range
        if !self.is_active_at_frame(frame) {
            return None;
        }

        let current_kf = self.keyframes
            .iter()
            .rev()
            .find(|kf| kf.frame <= frame);

        current_kf.map(|kf| kf.rotation)
    }

    pub fn get_delta_at_frame(
        &self,
        frame: u32,
        base_rotation: f64,
    ) -> Option<f64> {
        let r = self.get_rotation_at_frame(frame)? as f64;
        Some(r - base_rotation)
    }
}

impl SpriteSkewKeyframes {
    pub fn new(channel_index: u16, tween_info: Option<TweenInfo>) -> Self {
        Self {
            channel: index_to_channel_number(channel_index),
            keyframes: Vec::new(),
            tween_info,
            intervals: Vec::new(),
        }
    }

    pub fn from_frame_data(
        frame_channel_data: &Vec<(u32, u16, ScoreFrameChannelData)>,
        channel_index: u16,
        intervals: Option<&Vec<FrameIntervalPrimary>>,
    ) -> Self {
        let Some(intervals) = intervals else {
            return Self::new(channel_index, None);
        };

        // Use first interval's tween_info for the struct (they should all be consistent)
        let tween_info = intervals.first().map(|i| i.tween_info.clone());
        let mut keyframes = Self::new(channel_index, tween_info);

        // Store the interval ranges
        keyframes.intervals = intervals.iter()
            .map(|i| (i.start_frame, i.end_frame))
            .collect();

        // Process each interval
        for interval in intervals {
            // Ignore single-frame intervals
            if interval.start_frame == interval.end_frame {
                continue;
            }

            // Skew tween not enabled in this interval
            if !interval.tween_info.is_skew_tweened() {
                continue;
            }

            let start_frame = interval.start_frame;
            let end_frame = interval.end_frame;

            // 1️⃣ Collect frames in Director frame range AND matching this channel
            let frames: Vec<_> = frame_channel_data
                .iter()
                .filter(|(frame_num, ch, _)| {
                    let director_frame = *frame_num + 1;
                    let in_range = director_frame >= start_frame && director_frame <= end_frame;
                    let matches_channel = *ch == channel_index;
                    in_range && matches_channel
                })
                .collect();

            if frames.is_empty() {
                continue;
            }

            // 2️⃣ Deduplicate by frame number (shouldn't be needed now, but keep for safety)
            let mut dedup: HashMap<u32, &(u32, u16, ScoreFrameChannelData)> = HashMap::new();

            for frame in frames {
                let (frame_num, _, _) = frame;
                dedup.insert(*frame_num, frame);
            }

            let mut sorted_frames: Vec<_> = dedup.into_iter().map(|(_, v)| v.clone()).collect();
            sorted_frames.sort_by_key(|(frame_num, _, _)| *frame_num);

            // 3️⃣ Check if skew ACTUALLY animates in this interval
            if !has_real_animation::<Skew>(&sorted_frames) {
                continue;
            }

            // 4️⃣ Collect effective keyframes for this interval
            let property_frames = collect_property_keyframes::<Skew>(&sorted_frames);

            for (frame, Skew(skew)) in property_frames {
                keyframes.keyframes.push(SkewKeyframe {
                    frame,
                    skew,
                });
            }
        }

        // Sort all keyframes by frame number after collecting from all intervals
        keyframes.keyframes.sort_by_key(|kf| kf.frame);

        // 5️⃣ Debug output
        if !keyframes.keyframes.is_empty() {
            web_sys::console::log_1(&format!(
                "↔️ Channel {} has {} skew keyframes across {} intervals",
                keyframes.channel,
                keyframes.keyframes.len(),
                intervals.len()
            ).into());

            for kf in &keyframes.keyframes {
                web_sys::console::log_1(
                    &format!("   Frame {}: {:.2}°", kf.frame, kf.skew).into(),
                );
            }
        }

        keyframes
    }

    pub fn get_skew_at_frame(&self, frame: u32) -> Option<f64> {
        if self.keyframes.is_empty() {
            return None;
        }

        // Check if frame is within the active range
        if !self.is_active_at_frame(frame) {
            return None;
        }

        let current_kf = self.keyframes
            .iter()
            .rev()
            .find(|kf| kf.frame <= frame);

        current_kf.map(|kf| kf.skew)
    }

    pub fn get_delta_at_frame(
        &self,
        frame: u32,
        base_skew: f64,
    ) -> Option<f64> {
        let s = self.get_skew_at_frame(frame)?;
        Some(s - base_skew)
    }
}

impl SpritePathKeyframes {
    pub fn new(channel_index: u16, tween_info: Option<TweenInfo>) -> Self {
        Self {
            channel: index_to_channel_number(channel_index),
            keyframes: Vec::new(),
            tween_info,
            intervals: Vec::new(),
        }
    }

    pub fn from_frame_data(
        frame_channel_data: &Vec<(u32, u16, ScoreFrameChannelData)>,
        channel_index: u16,
        intervals: Option<&Vec<FrameIntervalPrimary>>,
    ) -> Self {
        let Some(intervals) = intervals else {
            return Self::new(channel_index, None);
        };

        // Use first interval's tween_info for the struct (they should all be consistent)
        let tween_info = intervals.first().map(|i| i.tween_info.clone());
        let mut keyframes = Self::new(channel_index, tween_info);

        // Store the interval ranges
        keyframes.intervals = intervals.iter()
            .map(|i| (i.start_frame, i.end_frame))
            .collect();

        // Process each interval
        for interval in intervals {
            // Ignore single-frame intervals
            if interval.start_frame == interval.end_frame {
                continue;
            }

            // Path tween not enabled in this interval
            if !interval.tween_info.is_path_tweened() {
                continue;
            }

            let start_frame = interval.start_frame;
            let end_frame = interval.end_frame;

            // 1️⃣ Collect frames in Director frame range AND matching this channel
            let frames: Vec<_> = frame_channel_data
                .iter()
                .filter(|(frame_num, ch, _)| {
                    let director_frame = *frame_num + 1;
                    let in_range = director_frame >= start_frame && director_frame <= end_frame;
                    let matches_channel = *ch == channel_index;
                    in_range && matches_channel
                })
                .collect();

            if frames.is_empty() {
                continue;
            }

            // 2️⃣ Deduplicate by frame number (shouldn't be needed now, but keep for safety)
            let mut dedup: HashMap<u32, &(u32, u16, ScoreFrameChannelData)> = HashMap::new();

            for frame in frames {
                let (frame_num, _, _) = frame;
                dedup.insert(*frame_num, frame);
            }

            let mut sorted_frames: Vec<_> = dedup.into_iter().map(|(_, v)| v.clone()).collect();
            sorted_frames.sort_by_key(|(frame_num, _, _)| *frame_num);

            // 3️⃣ Check if position ACTUALLY animates in this interval
            if !has_real_animation::<Position>(&sorted_frames) {
                continue;
            }

            // 4️⃣ Collect effective keyframes for this interval
            let property_frames = collect_property_keyframes::<Position>(&sorted_frames);

            for (frame_idx, Position(x, y)) in property_frames {
                // Convert 0-based frame_idx to 1-based Director frame
                let director_frame = frame_idx + 1;
                keyframes.keyframes.push(PathKeyframe {
                    frame: director_frame,
                    x,
                    y,
                });
            }
        }

        // Sort all keyframes by frame number after collecting from all intervals
        keyframes.keyframes.sort_by_key(|kf| kf.frame);

        // 5️⃣ Debug output
        if !keyframes.keyframes.is_empty() {
            web_sys::console::log_1(&format!(
                "🛤️ Channel {} has {} path keyframes across {} intervals",
                keyframes.channel,
                keyframes.keyframes.len(),
                intervals.len()
            ).into());

            for kf in &keyframes.keyframes {
                web_sys::console::log_1(
                    &format!("   Frame {}: ({:?},{:?})", kf.frame, kf.x, kf.y).into(),
                );
            }
        }

        keyframes
    }

    pub fn get_position_at_frame(&self, frame: u32) -> Option<(i16, i16)> {
        if self.keyframes.is_empty() {
            return None;
        }

        // Check if frame is within the active range
        if !self.is_active_at_frame(frame) {
            return None;
        }

        let current_kf = self.keyframes
            .iter()
            .rev()
            .find(|kf| kf.frame <= frame);

        current_kf.map(|kf| (kf.x, kf.y))
    }

    pub fn get_delta_at_frame(
        &self,
        frame: u32,
        base_x: i32,
        base_y: i32,
    ) -> Option<(i32, i32)> {
        let (x, y) = self.get_position_at_frame(frame)?;
        Some((
            x as i32 - base_x as i32,
            y as i32 - base_y as i32,
        ))
    }
}

impl SpriteSizeKeyframes {
    pub fn new(channel_index: u16, tween_info: Option<TweenInfo>) -> Self {
        Self {
            channel: index_to_channel_number(channel_index),
            keyframes: Vec::new(),
            tween_info,
            intervals: Vec::new(),
        }
    }

    pub fn from_frame_data(
        frame_channel_data: &Vec<(u32, u16, ScoreFrameChannelData)>,
        channel_index: u16,
        intervals: Option<&Vec<FrameIntervalPrimary>>,
    ) -> Self {
        let Some(intervals) = intervals else {
            return Self::new(channel_index, None);
        };

        // Use first interval's tween_info for the struct (they should all be consistent)
        let tween_info = intervals.first().map(|i| i.tween_info.clone());
        let mut keyframes = Self::new(channel_index, tween_info);

        // Store the interval ranges
        keyframes.intervals = intervals.iter()
            .map(|i| (i.start_frame, i.end_frame))
            .collect();

        // Process each interval
        for interval in intervals {
            // Ignore single-frame intervals
            if interval.start_frame == interval.end_frame {
                continue;
            }

            // Size tween not enabled in this interval
            if !interval.tween_info.is_size_tweened() {
                continue;
            }

            let start_frame = interval.start_frame;
            let end_frame = interval.end_frame;

            // 1️⃣ Collect frames in Director frame range AND matching this channel
            let frames: Vec<_> = frame_channel_data
                .iter()
                .filter(|(frame_num, ch, _)| {
                    let director_frame = *frame_num + 1;
                    let in_range = director_frame >= start_frame && director_frame <= end_frame;
                    let matches_channel = *ch == channel_index;
                    in_range && matches_channel
                })
                .collect();

            if frames.is_empty() {
                continue;
            }

            // 2️⃣ Deduplicate by frame number (shouldn't be needed now, but keep for safety)
            let mut dedup: HashMap<u32, &(u32, u16, ScoreFrameChannelData)> = HashMap::new();

            for frame in frames {
                let (frame_num, _, _) = frame;
                dedup.insert(*frame_num, frame);
            }

            let mut sorted_frames: Vec<_> = dedup.into_iter().map(|(_, v)| v.clone()).collect();
            sorted_frames.sort_by_key(|(frame_num, _, _)| *frame_num);

            // 3️⃣ Check if size ACTUALLY animates in this interval
            if !has_real_animation::<Size>(&sorted_frames) {
                continue;
            }

            // 4️⃣ Collect effective keyframes for this interval
            let property_frames = collect_property_keyframes::<Size>(&sorted_frames);

            for (frame, Size(width, height)) in property_frames {
                keyframes.keyframes.push(SizeKeyframe {
                    frame,
                    width: width as u16,
                    height: height as u16,
                });
            }
        }

        // Sort all keyframes by frame number after collecting from all intervals
        keyframes.keyframes.sort_by_key(|kf| kf.frame);

        // 5️⃣ Debug output
        if !keyframes.keyframes.is_empty() {
            web_sys::console::log_1(&format!(
                "📏 Channel {} has {} size keyframes across {} intervals",
                keyframes.channel,
                keyframes.keyframes.len(),
                intervals.len()
            ).into());

            for kf in &keyframes.keyframes {
                web_sys::console::log_1(
                    &format!("   Frame {}: {:?}x{:?}", kf.frame, kf.width, kf.height).into(),
                );
            }
        }

        keyframes
    }

    pub fn get_size_at_frame(&self, frame: u32) -> Option<(u16, u16)> {
        if self.keyframes.is_empty() {
            return None;
        }

        // Check if frame is within the active range
        if !self.is_active_at_frame(frame) {
            return None;
        }

        let current_kf = self.keyframes
            .iter()
            .rev()
            .find(|kf| kf.frame <= frame);

        current_kf.map(|kf| (kf.width, kf.height))
    }

    pub fn get_delta_at_frame(
        &self,
        frame: u32,
        base_x: i32,
        base_y: i32,
    ) -> Option<(i32, i32)> {
        let (x, y) = self.get_size_at_frame(frame)?;
        Some((
            x as i32 - base_x as i32,
            y as i32 - base_y as i32,
        ))
    }
}

impl SpriteForeColorKeyframes {
    pub fn new(channel_index: u16, tween_info: Option<TweenInfo>) -> Self {
        Self {
            channel: index_to_channel_number(channel_index),
            keyframes: Vec::new(),
            tween_info,
            intervals: Vec::new(),
        }
    }

    pub fn from_frame_data(
        frame_channel_data: &Vec<(u32, u16, ScoreFrameChannelData)>,
        channel_index: u16,
        intervals: Option<&Vec<FrameIntervalPrimary>>,
    ) -> Self {
        let Some(intervals) = intervals else {
            return Self::new(channel_index, None);
        };

        // Use first interval's tween_info for the struct (they should all be consistent)
        let tween_info = intervals.first().map(|i| i.tween_info.clone());
        let mut keyframes = Self::new(channel_index, tween_info);

        // Store the interval ranges
        keyframes.intervals = intervals.iter()
            .map(|i| (i.start_frame, i.end_frame))
            .collect();

        // Process each interval
        for interval in intervals {
            // Ignore single-frame intervals
            if interval.start_frame == interval.end_frame {
                continue;
            }

            // Forecolor tween not enabled in this interval
            if !interval.tween_info.is_forecolor_tweened() {
                continue;
            }

            let start_frame = interval.start_frame;
            let end_frame = interval.end_frame;

            // 1️⃣ Collect frames in Director frame range AND matching this channel
            let frames: Vec<_> = frame_channel_data
                .iter()
                .filter(|(frame_num, ch, _)| {
                    let director_frame = *frame_num + 1;
                    let in_range = director_frame >= start_frame && director_frame <= end_frame;
                    let matches_channel = *ch == channel_index;
                    in_range && matches_channel
                })
                .collect();

            if frames.is_empty() {
                continue;
            }

            // 2️⃣ Deduplicate by frame number (shouldn't be needed now, but keep for safety)
            let mut dedup: HashMap<u32, &(u32, u16, ScoreFrameChannelData)> = HashMap::new();

            for frame in frames {
                let (frame_num, _, _) = frame;
                dedup.insert(*frame_num, frame);
            }

            let mut sorted_frames: Vec<_> = dedup.into_iter().map(|(_, v)| v.clone()).collect();
            sorted_frames.sort_by_key(|(frame_num, _, _)| *frame_num);

            // 3️⃣ Check if fore color ACTUALLY animates in this interval
            if !has_real_animation::<ForeColor>(&sorted_frames) {
                continue;
            }

            // 4️⃣ Collect effective keyframes for this interval
            let property_frames = collect_property_keyframes::<ForeColor>(&sorted_frames);

            for (frame, ForeColor(color)) in property_frames {
                keyframes.keyframes.push(ColorKeyframe {
                    frame,
                    color,
                });
            }
        }

        // Sort all keyframes by frame number after collecting from all intervals
        keyframes.keyframes.sort_by_key(|kf| kf.frame);

        // 5️⃣ Debug output
        if !keyframes.keyframes.is_empty() {
            web_sys::console::log_1(&format!(
                "🎨 Channel {} has {} foreground color keyframes across {} intervals",
                keyframes.channel,
                keyframes.keyframes.len(),
                intervals.len()
            ).into());

            for kf in &keyframes.keyframes {
                web_sys::console::log_1(
                    &format!("   Frame {}: {:?}", kf.frame, kf.color).into(),
                );
            }
        }

        keyframes
    }

    pub fn get_color_at_frame(&self, frame: u32) -> Option<ColorRef> {
        if self.keyframes.is_empty() {
            return None;
        }

        // Check if frame is within the active range
        if !self.is_active_at_frame(frame) {
            return None;
        }

        let current_kf = self.keyframes
            .iter()
            .rev()
            .find(|kf| kf.frame <= frame);

        current_kf.map(|kf| kf.color.clone())
    }
}

impl SpriteBackColorKeyframes {
    pub fn new(channel_index: u16, tween_info: Option<TweenInfo>) -> Self {
        Self {
            channel: index_to_channel_number(channel_index),
            keyframes: Vec::new(),
            tween_info,
            intervals: Vec::new(),
        }
    }

    pub fn from_frame_data(
        frame_channel_data: &Vec<(u32, u16, ScoreFrameChannelData)>,
        channel_index: u16,
        intervals: Option<&Vec<FrameIntervalPrimary>>,
    ) -> Self {
        let Some(intervals) = intervals else {
            return Self::new(channel_index, None);
        };

        // Use first interval's tween_info for the struct (they should all be consistent)
        let tween_info = intervals.first().map(|i| i.tween_info.clone());
        let mut keyframes = Self::new(channel_index, tween_info);

        // Store the interval ranges
        keyframes.intervals = intervals.iter()
            .map(|i| (i.start_frame, i.end_frame))
            .collect();

        // Process each interval
        for interval in intervals {
            // Ignore single-frame intervals
            if interval.start_frame == interval.end_frame {
                continue;
            }

            // Backcolor tween not enabled in this interval
            if !interval.tween_info.is_backcolor_tweened() {
                continue;
            }

            let start_frame = interval.start_frame;
            let end_frame = interval.end_frame;

            // 1️⃣ Collect frames in Director frame range AND matching this channel
            let frames: Vec<_> = frame_channel_data
                .iter()
                .filter(|(frame_num, ch, _)| {
                    let director_frame = *frame_num + 1;
                    let in_range = director_frame >= start_frame && director_frame <= end_frame;
                    let matches_channel = *ch == channel_index;
                    in_range && matches_channel
                })
                .collect();

            if frames.is_empty() {
                continue;
            }

            // 2️⃣ Deduplicate by frame number (shouldn't be needed now, but keep for safety)
            let mut dedup: HashMap<u32, &(u32, u16, ScoreFrameChannelData)> = HashMap::new();

            for frame in frames {
                let (frame_num, _, _) = frame;
                dedup.insert(*frame_num, frame);
            }

            let mut sorted_frames: Vec<_> = dedup.into_iter().map(|(_, v)| v.clone()).collect();
            sorted_frames.sort_by_key(|(frame_num, _, _)| *frame_num);

            // 3️⃣ Check if back color ACTUALLY animates in this interval
            if !has_real_animation::<BackColor>(&sorted_frames) {
                continue;
            }

            // 4️⃣ Collect effective keyframes for this interval
            let property_frames = collect_property_keyframes::<BackColor>(&sorted_frames);

            for (frame, BackColor(color)) in property_frames {
                keyframes.keyframes.push(ColorKeyframe {
                    frame,
                    color,
                });
            }
        }

        // Sort all keyframes by frame number after collecting from all intervals
        keyframes.keyframes.sort_by_key(|kf| kf.frame);

        // 5️⃣ Debug output
        if !keyframes.keyframes.is_empty() {
            web_sys::console::log_1(&format!(
                "🖌️ Channel {} has {} background color keyframes across {} intervals",
                keyframes.channel,
                keyframes.keyframes.len(),
                intervals.len()
            ).into());

            for kf in &keyframes.keyframes {
                web_sys::console::log_1(
                    &format!("   Frame {}: {:?}", kf.frame, kf.color).into(),
                );
            }
        }

        keyframes
    }

    pub fn get_color_at_frame(&self, frame: u32) -> Option<ColorRef> {
        if self.keyframes.is_empty() {
            return None;
        }

        // Check if frame is within the active range
        if !self.is_active_at_frame(frame) {
            return None;
        }

        let current_kf = self.keyframes
            .iter()
            .rev()
            .find(|kf| kf.frame <= frame);

        current_kf.map(|kf| kf.color.clone())
    }
}

pub fn convert_blend_to_percentage(raw_blend: u8) -> u8 {
    if raw_blend == 0 {
        100
    } else if raw_blend == 255 {
        0
    } else {
        ((255.0 - raw_blend as f32) * 100.0 / 255.0) as u8
    }
}

pub fn build_blend_keyframes_cache(
    frame_channel_data: &Vec<(u32, u16, ScoreFrameChannelData)>,
    frame_intervals: &HashMap<u16, Vec<FrameIntervalPrimary>>,
) -> HashMap<u16, SpriteBlendKeyframes> {
    let mut keyframes_cache = HashMap::new();
    
    let mut channel_indices: Vec<u16> = frame_channel_data
        .iter()
        .map(|(_, ch, _)| *ch)
        .collect();
    channel_indices.sort();
    channel_indices.dedup();
    
    for channel_index in channel_indices {
        let channel_num = index_to_channel_number(channel_index);
        
        let intervals = frame_intervals.get(&channel_num);
        
        let keyframes = SpriteBlendKeyframes::from_frame_data(
            frame_channel_data, 
            channel_index,
            intervals
        );
        
        if !keyframes.keyframes.is_empty() {
            keyframes_cache.insert(keyframes.channel, keyframes);
        }
    }
    
    if keyframes_cache.is_empty() {
        web_sys::console::log_1(&format!("ℹ️ No blend keyframes found").into());
    } else {
        web_sys::console::log_1(&format!(
            "✅ Found blend keyframes in {} channels",
            keyframes_cache.len()
        ).into());
    }

    keyframes_cache
}

pub fn build_rotation_keyframes_cache(
    frame_channel_data: &Vec<(u32, u16, ScoreFrameChannelData)>,
    frame_intervals: &HashMap<u16, Vec<FrameIntervalPrimary>>,
) -> HashMap<u16, SpriteRotationKeyframes> {
    let mut keyframes_cache = HashMap::new();
    
    let mut channel_indices: Vec<u16> = frame_channel_data
        .iter()
        .map(|(_, ch, _)| *ch)
        .collect();
    channel_indices.sort();
    channel_indices.dedup();
    
    for channel_index in channel_indices {
        let channel_num = index_to_channel_number(channel_index);
        
        let intervals = frame_intervals.get(&channel_num);
        
        let keyframes = SpriteRotationKeyframes::from_frame_data(
            frame_channel_data, 
            channel_index,
            intervals
        );
        
        if !keyframes.keyframes.is_empty() {
            keyframes_cache.insert(keyframes.channel, keyframes);
        }
    }
    
    if keyframes_cache.is_empty() {
        web_sys::console::log_1(&format!("ℹ️ No rotation keyframes found").into());
    } else {
        web_sys::console::log_1(&format!(
            "✅ Found rotation keyframes in {} channels",
            keyframes_cache.len()
        ).into());
    }

    keyframes_cache
}

pub fn build_skew_keyframes_cache(
    frame_channel_data: &Vec<(u32, u16, ScoreFrameChannelData)>,
    frame_intervals: &HashMap<u16, Vec<FrameIntervalPrimary>>,
) -> HashMap<u16, SpriteSkewKeyframes> {
    let mut keyframes_cache = HashMap::new();
    
    let mut channel_indices: Vec<u16> = frame_channel_data
        .iter()
        .map(|(_, ch, _)| *ch)
        .collect();
    channel_indices.sort();
    channel_indices.dedup();
    
    for channel_index in channel_indices {
        let channel_num = index_to_channel_number(channel_index);
        
        let intervals = frame_intervals.get(&channel_num);
        
        let keyframes = SpriteSkewKeyframes::from_frame_data(
            frame_channel_data, 
            channel_index,
            intervals
        );
        
        if !keyframes.keyframes.is_empty() {
            keyframes_cache.insert(keyframes.channel, keyframes);
        }
    }
    
    if keyframes_cache.is_empty() {
        web_sys::console::log_1(&format!("ℹ️ No skew keyframes found").into());
    } else {
        web_sys::console::log_1(&format!(
            "✅ Found skew keyframes in {} channels",
            keyframes_cache.len()
        ).into());
    }

    keyframes_cache
}

pub fn build_path_keyframes_cache(
    frame_channel_data: &Vec<(u32, u16, ScoreFrameChannelData)>,
    frame_intervals: &HashMap<u16, Vec<FrameIntervalPrimary>>,
) -> HashMap<u16, SpritePathKeyframes> {
    let mut keyframes_cache = HashMap::new();
    
    let mut channel_indices: Vec<u16> = frame_channel_data
        .iter()
        .map(|(_, ch, _)| *ch)
        .collect();
    channel_indices.sort();
    channel_indices.dedup();
    
    for channel_index in channel_indices {
        let channel_num = index_to_channel_number(channel_index);
        
        let intervals = frame_intervals.get(&channel_num);
        
        let keyframes = SpritePathKeyframes::from_frame_data(
            frame_channel_data, 
            channel_index,
            intervals
        );
        
        if !keyframes.keyframes.is_empty() {
            keyframes_cache.insert(keyframes.channel, keyframes);
        }
    }
    
    if keyframes_cache.is_empty() {
        web_sys::console::log_1(&format!("ℹ️ No path keyframes found").into());
    } else {
        web_sys::console::log_1(&format!(
            "✅ Found path keyframes in {} channels",
            keyframes_cache.len()
        ).into());
    }

    keyframes_cache
}

pub fn build_size_keyframes_cache(
    frame_channel_data: &Vec<(u32, u16, ScoreFrameChannelData)>,
    frame_intervals: &HashMap<u16, Vec<FrameIntervalPrimary>>,
) -> HashMap<u16, SpriteSizeKeyframes> {
    let mut keyframes_cache = HashMap::new();
    
    let mut channel_indices: Vec<u16> = frame_channel_data
        .iter()
        .map(|(_, ch, _)| *ch)
        .collect();
    channel_indices.sort();
    channel_indices.dedup();
    
    for channel_index in channel_indices {
        let channel_num = index_to_channel_number(channel_index);
        
        let intervals = frame_intervals.get(&channel_num);
        
        let keyframes = SpriteSizeKeyframes::from_frame_data(
            frame_channel_data, 
            channel_index,
            intervals
        );
        
        if !keyframes.keyframes.is_empty() {
            keyframes_cache.insert(keyframes.channel, keyframes);
        }
    }
    
    if keyframes_cache.is_empty() {
        web_sys::console::log_1(&format!("ℹ️ No size keyframes found").into());
    } else {
        web_sys::console::log_1(&format!(
            "✅ Found size keyframes in {} channels",
            keyframes_cache.len()
        ).into());
    }

    keyframes_cache
}

pub fn build_fore_color_keyframes_cache(
    frame_channel_data: &Vec<(u32, u16, ScoreFrameChannelData)>,
    frame_intervals: &HashMap<u16, Vec<FrameIntervalPrimary>>,
) -> HashMap<u16, SpriteForeColorKeyframes> {   
    let mut keyframes_cache = HashMap::new();
    
    let mut channel_indices: Vec<u16> = frame_channel_data
        .iter()
        .map(|(_, ch, _)| *ch)
        .collect();
    channel_indices.sort();
    channel_indices.dedup();
    
    for channel_index in channel_indices {
        let channel_num = index_to_channel_number(channel_index);
        
        let intervals = frame_intervals.get(&channel_num);
        
        let keyframes = SpriteForeColorKeyframes::from_frame_data(
            frame_channel_data, 
            channel_index,
            intervals
        );
        
        if !keyframes.keyframes.is_empty() {
            keyframes_cache.insert(keyframes.channel, keyframes);
        }
    }
    
    if keyframes_cache.is_empty() {
        web_sys::console::log_1(&format!("ℹ️ No fore color keyframes found").into());
    } else {
        web_sys::console::log_1(&format!(
            "✅ Found fore color keyframes in {} channels",
            keyframes_cache.len()
        ).into());
    }

    keyframes_cache
}

pub fn build_back_color_keyframes_cache(
    frame_channel_data: &Vec<(u32, u16, ScoreFrameChannelData)>,
    frame_intervals: &HashMap<u16, Vec<FrameIntervalPrimary>>,
) -> HashMap<u16, SpriteBackColorKeyframes> {
    let mut keyframes_cache = HashMap::new();
    
    let mut channel_indices: Vec<u16> = frame_channel_data
        .iter()
        .map(|(_, ch, _)| *ch)
        .collect();
    channel_indices.sort();
    channel_indices.dedup();
    
    for channel_index in channel_indices {
        let channel_num = index_to_channel_number(channel_index);
        
        let intervals = frame_intervals.get(&channel_num);
        
        let keyframes = SpriteBackColorKeyframes::from_frame_data(
            frame_channel_data, 
            channel_index,
            intervals
        );
        
        if !keyframes.keyframes.is_empty() {
            keyframes_cache.insert(keyframes.channel, keyframes);
        }
    }
    
    if keyframes_cache.is_empty() {
        web_sys::console::log_1(&format!("ℹ️ No back color keyframes found").into());
    } else {
        web_sys::console::log_1(&format!(
            "✅ Found back color keyframes in {} channels",
            keyframes_cache.len()
        ).into());
    }

    keyframes_cache
}

/// Combined keyframes data for a channel
#[derive(Clone, Debug)]
pub struct ChannelKeyframes {
    pub channel: u16,
    pub blend: Option<SpriteBlendKeyframes>,
    pub rotation: Option<SpriteRotationKeyframes>,
    pub skew: Option<SpriteSkewKeyframes>,
    pub path: Option<SpritePathKeyframes>,
    pub size: Option<SpriteSizeKeyframes>,
    pub fore_color: Option<SpriteForeColorKeyframes>,
    pub back_color: Option<SpriteBackColorKeyframes>,
}

/// Build combined keyframes cache for all channels
pub fn build_all_keyframes_cache(
    frame_channel_data: &Vec<(u32, u16, ScoreFrameChannelData)>,
    frame_intervals: &Vec<(FrameIntervalPrimary, Option<crate::director::chunks::score::FrameIntervalSecondary>)>,
) -> HashMap<u16, ChannelKeyframes> {
    web_sys::console::log_1(&format!(
        "🎬 Building keyframes cache from {} frame entries and {} intervals",
        frame_channel_data.len(),
        frame_intervals.len()
    ).into());
    
    // Build map of channel → Vec<FrameIntervalPrimary> (channels can have multiple intervals!)
    let mut intervals_by_channel: HashMap<u16, Vec<FrameIntervalPrimary>> = HashMap::new();
    for (primary, _) in frame_intervals {
        let channel_num = index_to_channel_number(primary.channel_index as u16);
        intervals_by_channel.entry(channel_num).or_insert_with(Vec::new).push(primary.clone());
    }
    
    // Build keyframes with proper flag checking
    let blend_cache = build_blend_keyframes_cache(frame_channel_data, &intervals_by_channel);
    let rotation_cache = build_rotation_keyframes_cache(frame_channel_data, &intervals_by_channel);
    let skew_cache = build_skew_keyframes_cache(frame_channel_data, &intervals_by_channel);
    let path_cache = build_path_keyframes_cache(frame_channel_data, &intervals_by_channel);
    let size_cache = build_size_keyframes_cache(frame_channel_data, &intervals_by_channel);
    let fore_color_cache = build_fore_color_keyframes_cache(frame_channel_data, &intervals_by_channel);
    let back_color_cache = build_back_color_keyframes_cache(frame_channel_data, &intervals_by_channel);
    
    // Combine into ChannelKeyframes
    let mut combined_cache = HashMap::new();
    let mut channels: Vec<u16> = blend_cache.keys()
        .chain(rotation_cache.keys())
        .chain(skew_cache.keys())
        .chain(path_cache.keys())
        .chain(size_cache.keys())
        .chain(fore_color_cache.keys())
        .chain(back_color_cache.keys())
        .copied()
        .collect();
    channels.sort();
    channels.dedup();
    
    for channel in channels {
        combined_cache.insert(channel, ChannelKeyframes {
            channel,
            blend: blend_cache.get(&channel).cloned(),
            rotation: rotation_cache.get(&channel).cloned(),
            skew: skew_cache.get(&channel).cloned(),
            path: path_cache.get(&channel).cloned(),
            size: size_cache.get(&channel).cloned(),
            fore_color: fore_color_cache.get(&channel).cloned(),
            back_color: back_color_cache.get(&channel).cloned(),
        });
    }
    
    web_sys::console::log_1(&format!(
        "✅ Built keyframes cache for {} channels",
        combined_cache.len()
    ).into());
    
    combined_cache
}
