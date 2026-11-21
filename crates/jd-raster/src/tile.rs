//! Tiled pixel storage.
//!
//! A layer's pixels live in a grid of 256×256 tiles. Tiles are shared via `Arc`, so cloning a
//! layer (for undo snapshots, or to hand a frame to the renderer) costs one pointer per tile.
//! Editing will go through `Arc::make_mut`-style copy-on-write, and every new tile gets a fresh
//! [`TileId`] so the GPU cache knows exactly what to (re)upload.
//!
//! Format for now: RGBA8, sRGB-encoded, straight (non-premultiplied) alpha, the same as the
//! files we load. Higher bit depths come with milestone 2.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub const TILE_SIZE: u32 = 256;
pub const TILE_BYTES: usize = (TILE_SIZE * TILE_SIZE * 4) as usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileId(u64);

impl TileId {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

pub struct Tile {
    id: TileId,
    /// Always exactly `TILE_BYTES` long. Pixels outside the image (right/bottom edge tiles)
    /// are transparent.
    data: Box<[u8]>,
}

impl std::fmt::Debug for Tile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tile").field("id", &self.id).finish_non_exhaustive()
    }
}

impl Tile {
    pub fn new(data: Box<[u8]>) -> Self {
        assert_eq!(data.len(), TILE_BYTES, "tile data must be {TILE_SIZE}² RGBA8");
        Self { id: TileId::next(), data }
    }

    pub fn id(&self) -> TileId {
        self.id
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Mutable pixels. Gives the tile a new id, since anything cached for the old one
    /// (GPU textures, thumbnails) is now stale.
    pub fn data_mut(&mut self) -> &mut [u8] {
        self.id = TileId::next();
        &mut self.data
    }

    pub fn transparent() -> Self {
        Self::new(vec![0; TILE_BYTES].into_boxed_slice())
    }
}

impl Clone for Tile {
    /// A clone is a *different* tile as far as caches are concerned.
    fn clone(&self) -> Self {
        Self::new(self.data.clone())
    }
}

/// A layer's pixels. `None` tiles are fully transparent and cost nothing.
#[derive(Debug, Clone)]
pub struct TileGrid {
    width: u32,
    height: u32,
    cols: u32,
    rows: u32,
    tiles: Vec<Option<Arc<Tile>>>,
}

impl TileGrid {
    pub fn empty(width: u32, height: u32) -> Self {
        let cols = width.div_ceil(TILE_SIZE);
        let rows = height.div_ceil(TILE_SIZE);
        Self {
            width,
            height,
            cols,
            rows,
            tiles: vec![None; (cols * rows) as usize],
        }
    }

    /// Cuts a tightly packed RGBA8 buffer into tiles. Fully transparent tiles are dropped.
    pub fn from_rgba8(width: u32, height: u32, pixels: &[u8]) -> Self {
        assert_eq!(pixels.len(), width as usize * height as usize * 4);
        let mut grid = Self::empty(width, height);
        let ts = TILE_SIZE as usize;
        for ty in 0..grid.rows {
            for tx in 0..grid.cols {
                let x0 = (tx * TILE_SIZE) as usize;
                let y0 = (ty * TILE_SIZE) as usize;
                let w = ts.min(width as usize - x0);
                let h = ts.min(height as usize - y0);
                let mut data = vec![0u8; TILE_BYTES].into_boxed_slice();
                let mut any_visible = false;
                for row in 0..h {
                    let src = ((y0 + row) * width as usize + x0) * 4;
                    let src = &pixels[src..src + w * 4];
                    data[row * ts * 4..row * ts * 4 + w * 4].copy_from_slice(src);
                    any_visible |= src.chunks_exact(4).any(|p| p[3] != 0);
                }
                if any_visible {
                    grid.tiles[(ty * grid.cols + tx) as usize] = Some(Arc::new(Tile::new(data)));
                }
            }
        }
        grid
    }

    pub fn cols(&self) -> u32 {
        self.cols
    }

    pub fn rows(&self) -> u32 {
        self.rows
    }

    pub fn tile(&self, x: u32, y: u32) -> Option<&Arc<Tile>> {
        if x < self.cols && y < self.rows { self.tiles[(y * self.cols + x) as usize].as_ref() } else { None }
    }

    /// The tile at (x, y) for writing: created transparent if empty, copied if shared with
    /// another layer or an undo snapshot (copy-on-write), and given a fresh id.
    pub fn tile_mut(&mut self, x: u32, y: u32) -> Option<&mut Tile> {
        if x >= self.cols || y >= self.rows {
            return None;
        }
        let slot = &mut self.tiles[(y * self.cols + x) as usize];
        let tile = Arc::make_mut(slot.get_or_insert_with(|| Arc::new(Tile::transparent())));
        tile.data_mut();
        Some(tile)
    }

    /// Writes one pixel (RGBA8 sRGB, straight alpha). Writing transparent into an empty tile
    /// is a no-op.
    pub fn set_pixel(&mut self, x: u32, y: u32, px: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let (tx, ty) = (x / TILE_SIZE, y / TILE_SIZE);
        if px[3] == 0 && self.tile(tx, ty).is_none() {
            return;
        }
        let i = (((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) * 4) as usize;
        if let Some(tile) = self.tile_mut(tx, ty) {
            tile.data_mut()[i..i + 4].copy_from_slice(&px);
        }
    }

    /// Writes a tightly packed RGBA8 block at (x0, y0), clipped to the grid. Fully transparent
    /// pixels landing on empty tiles don't create tiles.
    pub fn write_region(&mut self, x0: u32, y0: u32, w: u32, h: u32, rgba: &[u8]) {
        let (x1, y1) = ((x0 + w).min(self.width), (y0 + h).min(self.height));
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        for ty in y0 / TILE_SIZE..=(y1 - 1) / TILE_SIZE {
            for tx in x0 / TILE_SIZE..=(x1 - 1) / TILE_SIZE {
                let (lx0, ly0) = (x0.max(tx * TILE_SIZE), y0.max(ty * TILE_SIZE));
                let (lx1, ly1) = (x1.min((tx + 1) * TILE_SIZE), y1.min((ty + 1) * TILE_SIZE));
                let any = (ly0..ly1).any(|y| {
                    let r = (((y - y0) * w + (lx0 - x0)) * 4) as usize;
                    rgba[r..r + ((lx1 - lx0) * 4) as usize].chunks_exact(4).any(|p| p[3] != 0)
                });
                if !any && self.tile(tx, ty).is_none() {
                    continue;
                }
                let Some(tile) = self.tile_mut(tx, ty) else { continue };
                let data = tile.data_mut();
                for y in ly0..ly1 {
                    let src = (((y - y0) * w + (lx0 - x0)) * 4) as usize;
                    let dst = (((y % TILE_SIZE) * TILE_SIZE + lx0 % TILE_SIZE) * 4) as usize;
                    let n = ((lx1 - lx0) * 4) as usize;
                    data[dst..dst + n].copy_from_slice(&rgba[src..src + n]);
                }
            }
        }
    }

    /// Places a tile at grid position (x, y). Out-of-range positions are ignored.
    pub fn set_tile(&mut self, x: u32, y: u32, tile: Option<Arc<Tile>>) {
        if x < self.cols && y < self.rows {
            self.tiles[(y * self.cols + x) as usize] = tile;
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Non-empty tiles with their grid coordinates.
    pub fn iter(&self) -> impl Iterator<Item = (u32, u32, &Arc<Tile>)> {
        self.tiles.iter().enumerate().filter_map(|(i, t)| {
            let i = i as u32;
            t.as_ref().map(|t| (i % self.cols, i / self.cols, t))
        })
    }

    /// Reads one pixel (RGBA8 sRGB, straight alpha). Out of bounds or empty → transparent.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0; 4];
        }
        let (tx, ty) = (x / TILE_SIZE, y / TILE_SIZE);
        match &self.tiles[(ty * self.cols + tx) as usize] {
            Some(tile) => {
                let i = (((y % TILE_SIZE) * TILE_SIZE + x % TILE_SIZE) * 4) as usize;
                tile.data()[i..i + 4].try_into().expect("4 bytes")
            }
            None => [0; 4],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(w: u32, h: u32) -> Vec<u8> {
        (0..w * h)
            .flat_map(|i| [(i % 251) as u8, (i % 13) as u8, 7, 255])
            .collect()
    }

    #[test]
    fn roundtrips_pixels_across_tile_edges() {
        let (w, h) = (600, 300);
        let px = image(w, h);
        let grid = TileGrid::from_rgba8(w, h, &px);
        for (x, y) in [(0, 0), (255, 0), (256, 0), (599, 299), (300, 257)] {
            let i = ((y * w + x) * 4) as usize;
            assert_eq!(grid.pixel(x, y), px[i..i + 4], "at {x},{y}");
        }
        assert_eq!(grid.iter().count(), 3 * 2);
    }

    #[test]
    fn transparent_tiles_are_skipped() {
        let (w, h) = (512, 256);
        let mut px = image(w, h);
        // Clear the right tile entirely.
        for y in 0..h {
            for x in 256..w {
                px[((y * w + x) * 4 + 3) as usize] = 0;
            }
        }
        let grid = TileGrid::from_rgba8(w, h, &px);
        let coords: Vec<_> = grid.iter().map(|(x, y, _)| (x, y)).collect();
        assert_eq!(coords, [(0, 0)]);
        assert_eq!(grid.pixel(300, 10), [0; 4]);
    }

    #[test]
    fn clones_get_new_ids() {
        let t = Tile::new(vec![0; TILE_BYTES].into_boxed_slice());
        assert_ne!(t.id(), t.clone().id());
    }
}
