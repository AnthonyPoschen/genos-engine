//! Resident shape blocks. Identical shapes share one span and the same offset.
//!
//! A level uploads a block once. A later frame reuses that offset. A shape with
//! no users returns to the free list. The free span stays untouched until the
//! in-flight frames can no longer read it.

use crate::pack::GpuVertex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub offset: u32,
    pub count: u32,
}

struct Live {
    key: u64,
    offset: u32,
    count: u32,
    used: bool,
}

struct Free {
    offset: u32,
    count: u32,
    /// Frames to wait before a new shape may overwrite this span.
    cool: u32,
}

/// One CPU copy of the shape pool. The GPU buffer follows `dirty` ranges.
#[derive(Default)]
pub struct MeshPool {
    verts: Vec<GpuVertex>,
    live: Vec<Live>,
    free: Vec<Free>,
    dirty: Vec<(u32, u32)>,
}

impl MeshPool {
    pub fn begin_frame(&mut self) {
        for block in &mut self.live {
            block.used = false;
        }
        self.dirty.clear();
    }

    pub fn contains(&self, key: u64) -> bool {
        self.live.iter().any(|block| block.key == key)
    }

    /// Drop one live block. The span can be reused on a later frame.
    pub fn release(&mut self, key: u64) {
        let Some(index) = self.live.iter().position(|block| block.key == key) else {
            return;
        };
        let block = self.live.swap_remove(index);
        self.free.push(Free {
            offset: block.offset,
            count: block.count,
            cool: 2,
        });
    }

    /// Return the resident block for `key`. Copy `verts` only when the key is new.
    pub fn use_mesh(&mut self, key: u64, verts: &[GpuVertex]) -> Option<Block> {
        if let Some(block) = self.live.iter_mut().find(|block| block.key == key) {
            block.used = true;
            return Some(Block {
                offset: block.offset,
                count: block.count,
            });
        }
        if verts.is_empty() {
            return None;
        }
        let count = verts.len() as u32;
        let offset = self.alloc(count);
        self.verts[offset as usize..offset as usize + verts.len()].copy_from_slice(verts);
        self.live.push(Live {
            key,
            offset,
            count,
            used: true,
        });
        self.dirty.push((offset, count));
        Some(Block { offset, count })
    }

    pub fn finish_frame(&mut self) {
        for span in &mut self.free {
            if span.cool > 0 {
                span.cool -= 1;
            }
        }
        let mut index = 0;
        while index < self.live.len() {
            if self.live[index].used {
                index += 1;
                continue;
            }
            let block = self.live.swap_remove(index);
            self.free.push(Free {
                offset: block.offset,
                count: block.count,
                cool: 2,
            });
        }
    }

    pub fn dirty(&self) -> &[(u32, u32)] {
        &self.dirty
    }

    pub fn vertices(&self) -> &[GpuVertex] {
        &self.verts
    }

    fn alloc(&mut self, count: u32) -> u32 {
        if let Some(index) = self
            .free
            .iter()
            .position(|span| span.cool == 0 && span.count >= count)
        {
            let span = self.free.swap_remove(index);
            if span.count > count {
                self.free.push(Free {
                    offset: span.offset + count,
                    count: span.count - count,
                    cool: 0,
                });
            }
            return span.offset;
        }
        let offset = self.verts.len() as u32;
        self.verts
            .resize(self.verts.len() + count as usize, blank_vertex());
        offset
    }
}

fn blank_vertex() -> GpuVertex {
    GpuVertex {
        pos: [0.0; 3],
        albedo: [1.0; 3],
        normal: [0.0, 1.0, 0.0],
        shade: 1.0,
        uv: [-1.0, -1.0],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verts(count: usize) -> Vec<GpuVertex> {
        (0..count)
            .map(|index| {
                let mut vert = blank_vertex();
                vert.pos[0] = index as f32;
                vert
            })
            .collect()
    }

    #[test]
    fn identical_shapes_share_a_block_and_a_free_span_is_reused() {
        let mut pool = MeshPool::default();
        pool.begin_frame();
        let first = pool.use_mesh(1, &verts(3)).expect("first shape");
        let again = pool.use_mesh(1, &[]).expect("same shape");
        pool.finish_frame();
        assert_eq!(first, again);
        assert_eq!(first.offset, 0);
        assert_eq!(pool.dirty(), &[(0, 3)]);

        pool.begin_frame();
        let kept_first = pool.use_mesh(1, &[]).expect("same shape again");
        let second = pool.use_mesh(2, &verts(2)).expect("second shape");
        pool.finish_frame();
        assert_eq!(kept_first.offset, 0);
        assert_eq!(second.offset, 3);
        assert!(pool.contains(1));
        assert_eq!(pool.dirty(), &[(3, 2)]);

        pool.begin_frame();
        let kept = pool.use_mesh(2, &[]).expect("kept shape");
        pool.finish_frame();
        assert_eq!(kept.offset, 3);
        assert!(!pool.contains(1));

        pool.begin_frame();
        pool.use_mesh(2, &[]).expect("stay live");
        pool.finish_frame();
        pool.begin_frame();
        pool.use_mesh(2, &[]).expect("stay live");
        pool.finish_frame();

        pool.begin_frame();
        let reused = pool.use_mesh(3, &verts(3)).expect("reused span");
        assert_eq!(reused.offset, 0);
        assert_eq!(pool.use_mesh(2, &[]).expect("live block").offset, 3);
    }
}
