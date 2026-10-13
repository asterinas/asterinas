use core::arch::asm;
use crate::prelude::*;
use crate::mm::*;

// 页大小常量
const PAGE_SIZE: usize = 4096;
const PAGE_SHIFT: usize = 12;

// Sv48x4 相关常量
const VPN_BITS: usize = 9;
const PPN_BITS: usize = 44;
const GSTAGE_LEVELS: usize = 4;  // Sv48x4有4级页表

// 页表项标志位 (根据RISC-V特权规范)
const PTE_V: u64 = 1 << 0;  // Valid
const PTE_R: u64 = 1 << 1;  // Read
const PTE_W: u64 = 1 << 2;  // Write
const PTE_X: u64 = 1 << 3;  // Execute
const PTE_U: u64 = 1 << 4;  // User (G-stage中未使用)
const PTE_G: u64 = 1 << 5;  // Global
const PTE_A: u64 = 1 << 6;  // Accessed
const PTE_D: u64 = 1 << 7;  // Dirty

// 物理地址位掩码 (Sv48x4: 44位PPN + 12位偏移 = 56位物理地址)
const PTE_PPN_MASK: u64 = ((1u64 << PPN_BITS) - 1) << 10;
const PTE_ADDR_MASK: u64 = PTE_PPN_MASK | 0xFFF;

/// G-stage页表结构
pub(crate) struct GStagePageTable {
    root: u64,  // 根页表指针
    root_paddr: u64,
    frames: Vec<Frame<()>>,
    seg: Vec<Segment<()>>,
}

/// 页表错误类型
#[derive(Debug)]
pub(crate) enum PageTableError {
    OutOfMemory,
    InvalidAddress,
    AlreadyMapped,
    NotMapped,
    InvalidAlignment,
}

impl GStagePageTable {
    /// 创建新的G-stage页表
    pub(crate) fn new() -> Self {
        if let Ok(seg) = FrameAllocOptions::new().alloc_segment(4) {
            let root_paddr = seg.paddr() as u64;
            let root = paddr_to_vaddr(root_paddr as usize) as u64;
        
            // 清零页表
            unsafe {
                core::ptr::write_bytes(root as *mut u64, 0, 4 * PAGE_SIZE / 8);
            }

            let mut segv = Vec::new();
            segv.push(seg);
        
            Self { root_paddr,  root , frames: Vec::new(), seg: segv}
        } else {
            Self { root_paddr:0,  root:0, frames: Vec::new(), seg: Vec::new()}
        }
    }

    fn alloc_page(&mut self) -> (u64, u64) {
        if let Ok(frame) = FrameAllocOptions::new().alloc_frame() {
            let ret_va = paddr_to_vaddr(frame.paddr()) as u64;
            let ret_pa = frame.paddr() as u64;
            self.frames.push(frame);
            return (ret_va, ret_pa);
        }
        (0,0)
    }
    
    /// 获取根页表的物理地址（用于填入hgatp寄存器）
    pub(crate) fn get_root(&self) -> u64 {
        self.root_paddr
    }

    /// 映射一个页面
    /// 
    /// # 参数
    /// - `gpa`: Guest物理地址（要映射的地址）
    /// - `hpa`: Host物理地址（映射到的地址）
    /// - `size`: 映射大小（字节）
    /// - `flags`: 页表项标志
    pub(crate) fn map(
        &mut self,
        gpa: u64,
        hpa: u64,
        size: usize,
        flags: u64,
    ) -> Result<()> {
        let num_pages = size / PAGE_SIZE;
        let pte_flags = flags | PTE_V | PTE_A | PTE_D | PTE_X | PTE_U;
        
        for i in 0..num_pages {
            let offset = (i * PAGE_SIZE) as u64;
            self.map_page(gpa + offset, hpa + offset, pte_flags)?;
        }
        
        Ok(())
    }
    
    /// 映射单个页面
    fn map_page(
        &mut self,
        gpa: u64,
        hpa: u64,
        flags: u64,
    ) -> Result<()> {
        // 提取VPN (Sv48x4使用48位VPN，分为4级，第1级11位，后三级9位)
        let vpn = [
            (gpa >> (PAGE_SHIFT + 0 * VPN_BITS)) & 0x1FF,
            (gpa >> (PAGE_SHIFT + 1 * VPN_BITS)) & 0x1FF,
            (gpa >> (PAGE_SHIFT + 2 * VPN_BITS)) & 0x1FF,
            (gpa >> (PAGE_SHIFT + 3 * VPN_BITS)) & 0x7FF,
        ];
        
        let mut table = self.root as *mut u64;
        
        // 遍历前3级页表
        for level in (1..GSTAGE_LEVELS).rev() {
            let idx = vpn[level] as usize;
            let pte_ptr = unsafe { table.add(idx) };
            let pte = unsafe { core::ptr::read_volatile(pte_ptr) };
            
            if pte & PTE_V == 0 {
                // 分配新页表
                let (new_table_va, new_table_pa) = self.alloc_page();
                let new_table = new_table_va as *mut u64;
                
                // 清零新页表
                unsafe {
                    core::ptr::write_bytes(new_table, 0, PAGE_SIZE / 8);
                }
                
                // 设置页表项指向新页表
                let new_pte = ((new_table_pa as u64) >> PAGE_SHIFT << 10) | PTE_V;
                unsafe {
                    core::ptr::write_volatile(pte_ptr, new_pte);
                }
                
                table = new_table;
            } else {
                // 获取下一级页表地址
                let next_table = ((pte & PTE_PPN_MASK) >> 10) << PAGE_SHIFT;
                table = paddr_to_vaddr(next_table as usize) as *mut u64;
            }
        }
        
        // 最后一级页表
        let idx = vpn[0] as usize;
        let pte_ptr = unsafe { table.add(idx) };
        let _pte = unsafe { core::ptr::read_volatile(pte_ptr) };
        
        // 设置页表项
        let new_pte = ((hpa >> PAGE_SHIFT) << 10) | flags;
        unsafe {
            core::ptr::write_volatile(pte_ptr, new_pte);
        }
        
        Ok(())
    }
    
    /// 解除映射
    /// 
    /// # 参数
    /// - `gpa`: Guest物理地址
    /// - `size`: 解除映射的大小（字节）
    pub(crate) fn unmap(&mut self, gpa: u64, size: usize) -> Result<usize> {
        let num_pages = size / PAGE_SIZE;
        
        for i in 0..num_pages {
            let offset = (i * PAGE_SIZE) as u64;
            let _ = self.unmap_page(gpa + offset);
        }
        
        Ok(num_pages)
    }
    
    /// 解除单个页面的映射
    fn unmap_page(&mut self, gpa: u64) -> Result<(), PageTableError> {
        // 提取VPN
        let vpn = [
            (gpa >> (PAGE_SHIFT + 0 * VPN_BITS)) & 0x1FF,
            (gpa >> (PAGE_SHIFT + 1 * VPN_BITS)) & 0x1FF,
            (gpa >> (PAGE_SHIFT + 2 * VPN_BITS)) & 0x1FF,
            (gpa >> (PAGE_SHIFT + 3 * VPN_BITS)) & 0x7FF,
        ];
        
        let mut table = self.root as *mut u64;
        let mut tables_to_free = [core::ptr::null_mut(); GSTAGE_LEVELS - 1];
        let mut free_count = 0;
        
        // 遍历前3级页表
        for level in (1..GSTAGE_LEVELS).rev() {
            let idx = vpn[level] as usize;
            let pte_ptr = unsafe { table.add(idx) };
            let pte = unsafe { core::ptr::read_volatile(pte_ptr) };
            
            if pte & PTE_V == 0 {
                return Err(PageTableError::NotMapped);
            }
            
            // 获取下一级页表地址
            let next_table = ((pte & PTE_PPN_MASK) >> 10) << PAGE_SHIFT;
            tables_to_free[free_count] = table;
            free_count += 1;
            table = next_table as *mut u64;
        }
        
        // 最后一级页表
        let idx = vpn[0] as usize;
        let pte_ptr = unsafe { table.add(idx) };
        let pte = unsafe { core::ptr::read_volatile(pte_ptr) };
        
        if pte & PTE_V == 0 {
            return Err(PageTableError::NotMapped);
        }
        
        // 清除页表项
        unsafe {
            core::ptr::write_volatile(pte_ptr, 0);
            // 刷新TLB（这里需要使用sfence.vma或hfence.gvma）
            asm!("hfence.gvma");
        }
        
        // TODO: 这里可以添加释放空页表的逻辑
        // 需要检查页表是否为空，如果为空则释放
        
        Ok(())
    }
}

/// 页表项访问辅助函数
impl GStagePageTable {
    /// 读取页表项
    fn read_pte(&self, table: *const u64, index: usize) -> u64 {
        unsafe { core::ptr::read_volatile(table.add(index)) }
    }
    
    /// 写入页表项
    fn write_pte(&self, table: *mut u64, index: usize, value: u64) {
        unsafe { core::ptr::write_volatile(table.add(index), value) }
    }
    
    /// 检查页表项是否有效
    fn is_valid(pte: u64) -> bool {
        pte & PTE_V != 0
    }
    
    /// 检查是否是大页映射
    fn is_leaf(pte: u64) -> bool {
        pte & (PTE_R | PTE_W | PTE_X) != 0
    }
}
