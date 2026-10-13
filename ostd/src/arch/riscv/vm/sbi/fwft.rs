use super::{KvmVcpuSbiExtension, KvmVcpuSbiReturn};
use crate::arch::vm::vcpu::VcpuArchInner;
use super::KVM_REG_RISCV_SBI_FWFT;
use super::VcpuSbiContext;
use alloc::vec::Vec;
use alloc::vec;

const SBI_EXT_FWFT: usize = 0x46574654;

pub(super) const VCPU_SBI_EXT_FWFT: KvmVcpuSbiExtension = KvmVcpuSbiExtension {
    extid_start: SBI_EXT_FWFT,
    extid_end: SBI_EXT_FWFT,
    default_disabled: false,
    handler: &sbi_ext_fwft_handler,
    probe: None,
    init: Some(&kvm_sbi_ext_fwft_init),
    deinit: Some(&sbi_ext_fwft_deinit),
    reset: Some(&kvm_sbi_ext_fwft_reset),
    state_reg_subtype: KVM_REG_RISCV_SBI_FWFT,
    get_state_reg_count: Some(&sbi_ext_fwft_get_state_reg_count),
    get_state_reg_id: Some(&sbi_ext_fwft_get_state_reg_id),
    get_state_reg: Some(&sbi_ext_fwft_get_state_reg),
    set_state_reg: Some(&sbi_ext_fwft_set_state_reg),
};

const SBI_FWFT_FEATURES: [KvmSbiFwftFeature; 2] = [
    KvmSbiFwftFeature {
        id: SbiFwftFeature::SbiFwftMisalignedExcDeleg,
        first_reg_num: 0,
        supported: &kvm_sbi_fwft_misaligned_delegation_supported,
        init: None,
        reset: None,
        set: None,
        get: None,
    },
    KvmSbiFwftFeature {
        id: SbiFwftFeature::SbiFwftPointerMaskingPmlen,
        first_reg_num: 3,
        supported: &kvm_sbi_fwft_pointer_masking_pmlen_supported,
        init: Some(&kvm_sbi_fwft_pointer_masking_pmlen_init),
        reset: None,
        set: None,
        get: None,
    },
];

pub(super) enum SbiFwftFeature {
	SbiFwftMisalignedExcDeleg		= 0x0,
	SbiFwftLandingPad			= 0x1,
	SbiFwftShadowStack			= 0x2,
	SbiFwftDoubleTrap			= 0x3,
	SbiFwftPteAdHwUpdating		= 0x4,
	SbiFwftPointerMaskingPmlen		= 0x5,
	SbiFwftLocalReservedStart		= 0x6,
	SbiFwftLocalReservedEnd		= 0x3fffffff,
	SbiFwftLocalPlatformStart		= 0x40000000,
	SbiFwftLocalPlatformEnd		= 0x7fffffff,

	SbiFwftGlobalReservedStart		= 0x80000000,
	SbiFwftGlobalReservedEnd		= 0xbfffffff,
	SbiFwftGlobalPlatformStart		= 0xc0000000,
	SbiFwftGlobalPlatformEnd		= 0xffffffff,
}

pub(super) type FwftSupportFunction = dyn Fn(&VcpuSbiContext) -> usize + Sync + Send + 'static;
pub(super) type FwftInitFunction = dyn Fn(&mut VcpuSbiContext) -> usize + Sync + Send + 'static;

pub(super) struct KvmSbiFwftFeature {
	/// Feature ID
	id: SbiFwftFeature,

	/// ONE_REG index of the first ONE_REG register
	first_reg_num: usize,

	/// Check if the feature is supported on the vcpu.
	/// This callback is optional, if not provided the feature is assumed to
	/// supported
	supported: &'static FwftSupportFunction,

	/// Probe and initialize the feature on the vcpu.
	/// This callback is optional. If provided, it will be called during
	/// vcpu initialization to probe the feature availability and perform
	/// any necessary initialization. Returns true if the feature is supported
	/// and initialized successfully, false otherwise.
	init: Option<&'static FwftInitFunction>,

	/// Reset the feature value irrespective whether feature is supported or not.
	/// This callback is mandatory
	reset: Option<&'static FwftInitFunction>,

	/// Set the feature value
	/// Return SBI_SUCCESS on success or an SBI error (SBI_ERR_*).
	/// This callback is mandatory
	set: Option<&'static FwftInitFunction>,

	/// Get the feature current value
	/// Return SBI_SUCCESS on success or an SBI error (SBI_ERR_*).
	/// This callback is mandatory
    get: Option<&'static FwftInitFunction>,
}

/// 1
pub(super) struct KvmSbiFwftConfig {
	feature: &'static KvmSbiFwftFeature,
	supported: bool,
	enabled: bool,
	flags: usize,
}

/// FWFT data structure per vcpu
#[repr(C)]
pub(super) struct KvmSbiFwft {
	configs: Vec<KvmSbiFwftConfig>,
	have_vs_pmlen_7: bool,
	have_vs_pmlen_16: bool,
}

impl KvmSbiFwft {
    /// 1
    pub(super) fn new() -> Self {
        Self {
            configs: vec![
                KvmSbiFwftConfig {
                    feature: &SBI_FWFT_FEATURES[0],
                    supported: false,
                    enabled: false,
                    flags: 0,
                },
                KvmSbiFwftConfig {
                    feature: &SBI_FWFT_FEATURES[1],
                    supported: false,
                    enabled: false,
                    flags: 0,
                },
            ],
            have_vs_pmlen_16: false,
            have_vs_pmlen_7: false,
        }
    }
}

fn sbi_ext_fwft_handler(_arch: &VcpuArchInner) -> KvmVcpuSbiReturn {
    KvmVcpuSbiReturn::default()
}

fn kvm_sbi_fwft_misaligned_delegation_supported(_ctx: &VcpuSbiContext) -> usize {
    0
}

fn kvm_sbi_fwft_pointer_masking_pmlen_supported(_ctx: &VcpuSbiContext) -> usize {
    0
}

fn kvm_sbi_fwft_pointer_masking_pmlen_init(_ctx: &mut VcpuSbiContext) -> usize {
    0
}

fn kvm_sbi_ext_fwft_init(ctx: &mut VcpuSbiContext) -> usize {
    for i in 0..2 {
        let (supported_fn, init_fn) = {
            let feature = &ctx.fwft_context.configs[i].feature;
            (feature.supported, feature.init)
        };

        let mut supported = supported_fn(ctx) == 1;
        if supported {
            if let Some(init) = init_fn {
                supported = init(ctx) == 1;
            }
        }

        let conf = &mut ctx.fwft_context.configs[i];
        conf.supported = supported;
        conf.enabled = supported;
    }
    0
}

fn sbi_ext_fwft_deinit() -> KvmVcpuSbiReturn {
    KvmVcpuSbiReturn::default()
}

fn kvm_sbi_ext_fwft_reset(_ctx: &mut VcpuSbiContext) -> usize {
    0
}

fn sbi_ext_fwft_get_state_reg_count() -> usize {
    0
}

fn sbi_ext_fwft_get_state_reg_id(_index: usize) -> usize {
    0
}

fn sbi_ext_fwft_get_state_reg() -> KvmVcpuSbiReturn {
    KvmVcpuSbiReturn::default()
}

fn sbi_ext_fwft_set_state_reg() -> KvmVcpuSbiReturn {
    KvmVcpuSbiReturn::default()
}
