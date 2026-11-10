//! Render-pass dependencies for a depth clear.
//!
//! `vkCmdBeginRenderPass` performs the attachment layout transition and, when
//! the depth load op is CLEAR, a depth write in `EARLY_FRAGMENT_TESTS`. Those
//! are two writes of the same attachment. The external → subpass 0 dependency
//! has to include early and late fragment tests and a depth write, or sync
//! validation reports WRITE_AFTER_WRITE at the begin.

/// Stage and access masks of one `VkSubpassDependency`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DepMasks {
    pub src_stage: u32,
    pub dst_stage: u32,
    pub src_access: u32,
    pub dst_access: u32,
}

/// `VK_PIPELINE_STAGE_EARLY_FRAGMENT_TESTS_BIT`.
pub(crate) const EARLY_FRAGMENT_TESTS: u32 = 0x0100;
/// `VK_PIPELINE_STAGE_LATE_FRAGMENT_TESTS_BIT`.
pub(crate) const LATE_FRAGMENT_TESTS: u32 = 0x0200;
/// `VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT`.
pub(crate) const COLOR_ATTACHMENT_OUTPUT: u32 = 0x0400;
/// `VK_PIPELINE_STAGE_FRAGMENT_SHADER_BIT`.
pub(crate) const FRAGMENT_SHADER: u32 = 0x0080;
/// `VK_PIPELINE_STAGE_TRANSFER_BIT`.
pub(crate) const TRANSFER: u32 = 0x1000;

/// `VK_ACCESS_SHADER_READ_BIT`.
pub(crate) const SHADER_READ: u32 = 0x0020;
/// `VK_ACCESS_COLOR_ATTACHMENT_READ_BIT`.
pub(crate) const COLOR_READ: u32 = 0x0080;
/// `VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT`.
pub(crate) const COLOR_WRITE: u32 = 0x0100;
/// `VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_READ_BIT`.
pub(crate) const DEPTH_READ: u32 = 0x0200;
/// `VK_ACCESS_DEPTH_STENCIL_ATTACHMENT_WRITE_BIT`.
pub(crate) const DEPTH_WRITE: u32 = 0x0400;
/// `VK_ACCESS_TRANSFER_READ_BIT`.
pub(crate) const TRANSFER_READ: u32 = 0x0800;

/// External → subpass 0, including the depth clear.
///
/// `src_*` is whatever the previous use of the attachments was. Early and late
/// fragment tests are added so a prior depth write is ordered against the
/// transition. The destination is the clear itself: color output, early and
/// late fragment tests, and a depth write.
pub(crate) fn external_to_zero(src_stage: u32, src_access: u32, dst_color: u32) -> DepMasks {
    DepMasks {
        src_stage: src_stage | EARLY_FRAGMENT_TESTS | LATE_FRAGMENT_TESTS,
        dst_stage: COLOR_ATTACHMENT_OUTPUT | EARLY_FRAGMENT_TESTS | LATE_FRAGMENT_TESTS,
        src_access: src_access | DEPTH_WRITE,
        dst_access: dst_color | DEPTH_READ | DEPTH_WRITE,
    }
}

/// Subpass 0 → external. Depth writes in the pass become available afterwards.
pub(crate) fn zero_to_external(dst_stage: u32, dst_access: u32, src_color: u32) -> DepMasks {
    DepMasks {
        src_stage: COLOR_ATTACHMENT_OUTPUT | EARLY_FRAGMENT_TESTS | LATE_FRAGMENT_TESTS,
        dst_stage,
        src_access: src_color | DEPTH_WRITE,
        dst_access,
    }
}

/// Scene pass, external → 0. Color may be a transfer source from the filter,
/// or undefined before a clear. Depth starts undefined and is cleared.
pub(crate) fn scene_pass_in() -> DepMasks {
    external_to_zero(
        TRANSFER | COLOR_ATTACHMENT_OUTPUT,
        TRANSFER_READ | COLOR_WRITE,
        COLOR_READ | COLOR_WRITE,
    )
}

/// Scene pass, 0 → external. The filter reads the color attachment.
pub(crate) fn scene_pass_out() -> DepMasks {
    zero_to_external(TRANSFER, TRANSFER_READ, COLOR_WRITE)
}

/// Reflection pass, external → 0. The color image may be sampled or copied,
/// and the depth image is cleared or loaded.
pub(crate) fn reflect_pass_in() -> DepMasks {
    external_to_zero(
        FRAGMENT_SHADER | TRANSFER,
        SHADER_READ | TRANSFER_READ,
        COLOR_WRITE | DEPTH_WRITE,
    )
}

/// Reflection pass, 0 → external. Later passes sample or copy both attachments.
pub(crate) fn reflect_pass_out() -> DepMasks {
    zero_to_external(
        FRAGMENT_SHADER | TRANSFER,
        SHADER_READ | TRANSFER_READ,
        COLOR_WRITE | DEPTH_WRITE,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        reflect_pass_in, scene_pass_in, COLOR_ATTACHMENT_OUTPUT, DEPTH_WRITE, EARLY_FRAGMENT_TESTS,
        LATE_FRAGMENT_TESTS,
    };

    #[test]
    fn depth_clear_waits_for_the_layout_transition() {
        for dep in [scene_pass_in(), reflect_pass_in()] {
            assert_eq!(
                dep.dst_stage & EARLY_FRAGMENT_TESTS,
                EARLY_FRAGMENT_TESTS,
                "dst stage {:#x}",
                dep.dst_stage
            );
            assert_eq!(
                dep.dst_stage & LATE_FRAGMENT_TESTS,
                LATE_FRAGMENT_TESTS,
                "dst stage {:#x}",
                dep.dst_stage
            );
            assert_eq!(
                dep.dst_stage & COLOR_ATTACHMENT_OUTPUT,
                COLOR_ATTACHMENT_OUTPUT,
                "dst stage {:#x}",
                dep.dst_stage
            );
            assert_eq!(
                dep.dst_access & DEPTH_WRITE,
                DEPTH_WRITE,
                "dst access {:#x}",
                dep.dst_access
            );
        }
    }
}
