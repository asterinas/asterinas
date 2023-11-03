
#[cfg(target_arch = "x86_64")]
pub fn le16_to_cpu(a:u16){
    a
}

#[cfg(target_arch = "x86_64")]
pub fn le32_to_cpu(a:u32){
    a
}

#[cfg(target_arch = "x86_64")]
pub fn le64_to_cpu(a:u64){
    a
}