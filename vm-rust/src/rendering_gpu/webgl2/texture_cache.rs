//! Texture caching for WebGL2 renderer
//!
//! Caches bitmap textures on the GPU to avoid re-uploading
//! unchanged bitmaps every frame.

use std::collections::HashMap;
use web_sys::WebGlTexture;

use crate::player::cast_lib::CastMemberRef;
use crate::player::sprite::ColorRef;

/// Cache key that includes member reference, ink mode, and colorize parameters
/// Different ink modes may need different textures because the matte mask computation
/// differs. For example, 32-bit bitmaps with ink 8 (Matte) need flood-fill matte computation,
/// but with other inks they use the bitmap's embedded alpha.
///
/// Colorize (foreColor/backColor) is now included because Director's colorize feature
/// remaps palette indices to interpolate between fore and back colors.
///
/// For ink 8 (Matte), the sprite's bgColor affects the matte computation for indexed bitmaps.
/// Canvas2D uses the sprite's bgColor as the background color for flood-fill matte computation,
/// so different sprites using the same bitmap with different bgColors need different textures.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextureCacheKey {
    pub member_ref: CastMemberRef,
    /// Ink mode - affects whether matte computation is applied (especially for 32-bit bitmaps)
    pub ink: i32,
    /// Colorize parameters (only used when has_fore_color or has_back_color is true)
    /// Format: (has_fore, has_back, fg_r, fg_g, fg_b, bg_r, bg_g, bg_b)
    /// Using Option to avoid cache misses when colorize isn't used
    pub colorize: Option<(bool, bool, u8, u8, u8, u8, u8, u8)>,
    /// Sprite's bgColor for ink 8 indexed bitmaps (affects matte computation)
    /// Only included when ink==8 and bitmap is indexed (depth <= 8)
    /// Using Option to avoid cache misses when not applicable
    pub sprite_bg_color: Option<(u8, u8, u8)>,
}

/// Cached texture information
pub struct CachedTexture {
    /// WebGL texture handle
    pub texture: WebGlTexture,
    /// Texture width
    pub width: u32,
    /// Texture height
    pub height: u32,
    /// Version counter for invalidation
    pub version: u32,
    /// Last frame this texture was used
    pub last_used_frame: u64,
}

/// LRU texture cache for bitmap textures
pub struct TextureCache {
    /// Cached textures by member reference + bgColor
    textures: HashMap<TextureCacheKey, CachedTexture>,
    /// Maximum number of textures to cache
    max_size: usize,
    /// Current frame counter for LRU tracking
    current_frame: u64,
}

impl TextureCache {
    /// Create a new texture cache with default max size
    pub fn new() -> Self {
        Self::with_max_size(256)
    }

    /// Create a new texture cache with specified max size
    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            textures: HashMap::new(),
            max_size,
            current_frame: 0,
        }
    }

    /// Get a cached texture for a cache key (member + bgColor)
    pub fn get(&mut self, key: &TextureCacheKey) -> Option<&CachedTexture> {
        if let Some(cached) = self.textures.get_mut(key) {
            cached.last_used_frame = self.current_frame;
            Some(cached)
        } else {
            None
        }
    }

    /// Insert or update a texture in the cache
    pub fn insert(
        &mut self,
        key: TextureCacheKey,
        texture: WebGlTexture,
        width: u32,
        height: u32,
        version: u32,
    ) {
        // Evict old entries if cache is full
        if self.textures.len() >= self.max_size {
            self.evict_lru();
        }

        self.textures.insert(
            key,
            CachedTexture {
                texture,
                width,
                height,
                version,
                last_used_frame: self.current_frame,
            },
        );
    }

    /// Check if a texture needs updating (version changed)
    pub fn needs_update(&self, key: &TextureCacheKey, version: u32) -> bool {
        match self.textures.get(key) {
            Some(cached) => cached.version != version,
            None => true,
        }
    }

    /// Remove a texture from the cache
    pub fn remove(&mut self, key: &TextureCacheKey) -> Option<CachedTexture> {
        self.textures.remove(key)
    }

    /// Clear all cached textures
    pub fn clear(&mut self) {
        self.textures.clear();
    }

    /// Advance to next frame (for LRU tracking)
    pub fn next_frame(&mut self) {
        self.current_frame += 1;
    }

    /// Get number of cached textures
    pub fn len(&self) -> usize {
        self.textures.len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }

    /// Check if a key exists in the cache (without updating last_used_frame)
    pub fn has(&self, key: &TextureCacheKey) -> bool {
        self.textures.contains_key(key)
    }

    /// Evict least recently used texture
    fn evict_lru(&mut self) {
        let oldest_key = self
            .textures
            .iter()
            .min_by_key(|(_, cached)| cached.last_used_frame)
            .map(|(key, _)| key.clone());

        if let Some(key) = oldest_key {
            self.textures.remove(&key);
        }
    }
}

impl Default for TextureCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Cache key for rendered text textures
/// Unlike bitmap textures which are cached by member_ref, text textures need to be
/// keyed by all the parameters that affect the rendered appearance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RenderedTextCacheKey {
    /// The member reference (for Field/Text/Font member)
    pub member_ref: CastMemberRef,
    /// The actual text content (hash or truncated string for memory efficiency)
    pub text_hash: u64,
    /// Ink mode
    pub ink: i32,
    /// Blend value
    pub blend: i32,
    /// Foreground color
    pub fg_color: ColorRef,
    /// Background color
    pub bg_color: ColorRef,
    /// Whether the field has keyboard focus (affects cursor rendering)
    pub has_focus: bool,
    /// Texture width (needed because sprite rect can change independently of text)
    pub width: u32,
    /// Texture height
    pub height: u32,
}

impl RenderedTextCacheKey {
    /// Create a new cache key with a hash of the text content
    pub fn new(
        member_ref: CastMemberRef,
        text: &str,
        ink: i32,
        blend: i32,
        fg_color: ColorRef,
        bg_color: ColorRef,
        width: u32,
        height: u32,
    ) -> Self {
        Self::new_with_focus(member_ref, text, ink, blend, fg_color, bg_color, false, width, height)
    }

    /// Create a new cache key with focus state (for Field members with cursor)
    pub fn new_with_focus(
        member_ref: CastMemberRef,
        text: &str,
        ink: i32,
        blend: i32,
        fg_color: ColorRef,
        bg_color: ColorRef,
        has_focus: bool,
        width: u32,
        height: u32,
    ) -> Self {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let text_hash = hasher.finish();

        Self {
            member_ref,
            text_hash,
            ink,
            blend,
            fg_color,
            bg_color,
            has_focus,
            width,
            height,
        }
    }
}

/// Cached rendered text texture information
pub struct CachedRenderedText {
    /// WebGL texture handle
    pub texture: WebGlTexture,
    /// Texture width
    pub width: u32,
    /// Texture height
    pub height: u32,
    /// Last frame this texture was used
    pub last_used_frame: u64,
}

/// LRU texture cache for rendered text textures
pub struct RenderedTextCache {
    /// Cached textures by text cache key
    textures: HashMap<RenderedTextCacheKey, CachedRenderedText>,
    /// Maximum number of textures to cache
    max_size: usize,
    /// Current frame counter for LRU tracking
    current_frame: u64,
}

impl RenderedTextCache {
    /// Create a new rendered text cache with default max size
    pub fn new() -> Self {
        Self::with_max_size(128)
    }

    /// Create a new rendered text cache with specified max size
    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            textures: HashMap::new(),
            max_size,
            current_frame: 0,
        }
    }

    /// Get a cached texture
    pub fn get(&mut self, key: &RenderedTextCacheKey) -> Option<&CachedRenderedText> {
        if let Some(cached) = self.textures.get_mut(key) {
            cached.last_used_frame = self.current_frame;
            Some(cached)
        } else {
            None
        }
    }

    /// Insert a texture into the cache
    pub fn insert(
        &mut self,
        key: RenderedTextCacheKey,
        texture: WebGlTexture,
        width: u32,
        height: u32,
    ) {
        // Evict old entries if cache is full
        if self.textures.len() >= self.max_size {
            self.evict_lru();
        }

        self.textures.insert(
            key,
            CachedRenderedText {
                texture,
                width,
                height,
                last_used_frame: self.current_frame,
            },
        );
    }

    /// Advance to next frame (for LRU tracking)
    pub fn next_frame(&mut self) {
        self.current_frame += 1;
    }

    /// Clear all cached textures
    pub fn clear(&mut self) {
        self.textures.clear();
    }

    /// Evict least recently used texture
    fn evict_lru(&mut self) {
        let oldest_key = self
            .textures
            .iter()
            .min_by_key(|(_, cached)| cached.last_used_frame)
            .map(|(key, _)| key.clone());

        if let Some(key) = oldest_key {
            self.textures.remove(&key);
        }
    }
}

impl Default for RenderedTextCache {
    fn default() -> Self {
        Self::new()
    }
}
