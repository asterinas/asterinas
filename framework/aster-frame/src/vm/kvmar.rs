use log::trace;
use super::{VmIo, PAGE_SIZE};
use crate::{prelude::*, Error};
use crate::{
    arch::mm::PageTableFlags,
    vm::{page_table::KERNEL_PAGE_TABLE, VmAllocOptions},
};

#[derive(Debug, Clone)]
pub struct Kvmar {
    // Ensure that base is aligned to PAGE_SIZE and size is a multiple of PAGE_SIZE (at least 2 times)
    start_vaddr:usize,
    // 包含了guard page
    size:usize,
    // except guard pages
    nframes:usize,
    // 如果没有commit page就会报错
    end_vaddr: Option<Vaddr>
}

impl Kvmar {

    pub fn new(start_vaddr:usize, size:usize) -> Result<Self> {
        // 处理一下start_addr不是pagesize对齐，以及size不是pagesize对齐
        let nframes = size / PAGE_SIZE - 1;
        Ok(Self {
            start_vaddr,
            size,
            nframes: nframes as usize,
            end_vaddr: None,
        })
    }

    pub fn new_with_commit_pages(start_vaddr: usize, size: usize, flags:PageTableFlags) -> Result<Self> {
        
        trace!("NEW VMAR");

        let mut new_kvmar = Kvmar::new(start_vaddr, size)?;
        trace!("NEW VMAR 2");
        let _ = new_kvmar.commit_pages(flags);

        Ok(new_kvmar)
    }

    pub fn end_vaddr(&self) -> Result<Vaddr> {
        // 还没有分配内存
        Ok(self.start_vaddr + self.size)
    }

    // pub fn guard_page(vaddr:usize, flags:PageTableFlags) -> PageTableFlags {
    //     // 好像不需要这个过程
    //     let mut page_table = KERNEL_PAGE_TABLE.get().unwrap().lock();
    //     // 原本的实现就是用一个 empty flag
    //     page_table.guard(vaddr, flags).unwrap()

    // }
    
    // pub fn guard_last_page(&self, flags:PageTableFlags) -> PageTableFlags {

    //     Self::guard_page(self.start_vaddr, flags)
    // }

    pub fn commit_pages(&mut self, flag:PageTableFlags) -> Result<()> {
        // 一定得是连续内存吗？
        // 一堆物理帧
        // 要求没有事先分配过,分配连续的
        // trace!(
        //     "yyyyyyyyyyyyyyyytttttttttttiiiiiiiiii"
        // );
        // 初始化会报一个非法访问的错误
        let frames = VmAllocOptions::new(self.nframes).uninit(true).alloc()?;
        // 
        // trace!(
        //     "yyyyyyyyyyyyyyyyttttttttttt"
        // );
        // 写页表,原本的代码逻辑页表只写了guard page这个，可能需要更详细的看代码？
        // 放弃线性映射的话
        // 启动的时候要跑哪个命令  debug_assert!(frame.start_paddr() < PHYS_MEM_BASE_VADDR);
        let mut kernel_pt = KERNEL_PAGE_TABLE.get().unwrap().lock();
        // trace!(
        //     "yyyyyyyyyfffffffffffffffffttt"
        // );
        let mut va = self.start_vaddr + PAGE_SIZE;

        for i in 0..self.nframes {
            if let Some(frame_ref) = frames.get(i) {
                // let frame = *frame_ref;
                trace!(
                    "Kvmar: Map vaddr:{:x?}, paddr:{:x?}, flags:{:x?}",
                    va,
                    frame_ref.start_paddr(),
                    flag
                );
                unsafe{ kernel_pt.map(va, frame_ref.start_paddr(), flag).unwrap();}
                trace!(
                    "Kvmar: IS:{:x?}?",
                    kernel_pt.flags(va)
                );
                va = va + PAGE_SIZE;
            } else {
                return Err(Error::NoMemory);
            }
        }
        Ok(())
    }
    
    pub fn decommit_pages(&self) -> Result<()> {
        let mut kernel_pt = KERNEL_PAGE_TABLE.get().unwrap().lock();
        let mut va = self.start_vaddr + PAGE_SIZE;
        for i in 0..self.nframes {
            unsafe{ kernel_pt.unmap(va).unwrap();}
            va = va + PAGE_SIZE;
        }
        // 这个帧怎么回收?
        Ok(())
    }

    pub fn get_start_vaddr(&self) -> usize {
        self.start_vaddr // PAGE_SIZE
    }

    pub fn get_nframes(&self) ->usize {
        self.nframes
    }

    pub fn clear(&self)->Result<()> {
        self.decommit_pages()
    }
}

impl Drop for Kvmar {
    fn drop(&mut self){
        let _ = self.clear();
    }
} 

// impl VmIo for Kvmar{
//     fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
//         Ok(())
//     }

//     fn write_bytes(&self, offset:usize, buf: &mut [u8]) -> Result<()> {
//         Ok(())
//     }
// }