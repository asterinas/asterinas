use alloc::string::ToString;
use component::ComponentInitError;
// use jinux_input::INPUT_COMPONENT;
use jinux_virtio::{self, device::socket::{header::VsockAddr, device::SocketDevice, manager::VsockConnectionManager, DEVICE_NAME}};
use log::{info, debug};

pub fn init() {
    // print all the input device to make sure input crate will compile
    for (name, _) in jinux_input::all_devices() {
        info!("Found Input device, name:{}", name);
    }
    // let _ = socket_device_client_test();
    // let _ = socket_device_server_test();
}

#[allow(unused)]
fn block_device_test() {
    for (_, device) in jinux_block::all_devices() {
        let mut write_buffer = [0u8; 512];
        let mut read_buffer = [0u8; 512];
        info!("write_buffer address:{:x}", write_buffer.as_ptr() as usize);
        info!("read_buffer address:{:x}", read_buffer.as_ptr() as usize);
        for i in 0..512 {
            for byte in write_buffer.iter_mut() {
                *byte = i as u8;
            }
            device.write_block(i as usize, &write_buffer);
            device.read_block(i as usize, &mut read_buffer);
            assert_eq!(write_buffer, read_buffer);
        }
        info!("block device test passed!");
    }
}


fn socket_device_client_test() -> Result<(),ComponentInitError> {
    let host_cid = 2;
    let guest_cid = 3;
    let host_port = 1234;
    let guest_port = 4321;
    let host_address = VsockAddr {
        cid: host_cid,
        port: host_port,
    };
    let hello_from_guest = "Hello from guest";
    let hello_from_host = "Hello from host";

    let device = jinux_virtio::device::socket::get_device(DEVICE_NAME).unwrap();
    assert_eq!(device.lock().guest_cid(),guest_cid);
    let mut socket = VsockConnectionManager::new(device);

    socket.connect(host_address, guest_port).unwrap();
    socket.wait_for_event().unwrap(); // wait for connect response
    socket.send(host_address,guest_port,hello_from_guest.as_bytes()).unwrap();
    debug!("The buffer {:?} is sent, start receiving",hello_from_guest.as_bytes());
    socket.wait_for_event().unwrap(); // wait for recv
    let mut buffer = [0u8; 64];
    let event = socket.recv(host_address, guest_port,&mut buffer).unwrap();
    assert_eq!(
        &buffer[0..hello_from_guest.len()],
        hello_from_guest.as_bytes()
    );

    socket.force_close(host_address,guest_port).unwrap();

    debug!("The final event: {:?}",event);
    Ok(())
}

pub fn socket_device_server_test() -> Result<(),ComponentInitError>{
    let host_cid = 2;
    let guest_cid = 3;
    let host_port = 63525137;
    let guest_port = 4321;
    let host_address = VsockAddr {
        cid: host_cid,
        port: host_port,
    };
    let hello_from_guest = "Hello from guest";
    let hello_from_host = "Hello from host";

    let device = jinux_virtio::device::socket::get_device(DEVICE_NAME).unwrap();
    assert_eq!(device.lock().guest_cid(),guest_cid);
    let mut socket = VsockConnectionManager::new(device);

    socket.listen(4321);
    socket.wait_for_event().unwrap(); // wait for connect request
    socket.wait_for_event().unwrap(); // wait for recv
    let mut buffer = [0u8; 64];
    let event = socket.recv(host_address, guest_port,&mut buffer).unwrap();
    assert_eq!(
        &buffer[0..hello_from_host.len()],
        hello_from_host.as_bytes()
    );

    debug!("The buffer {:?} is received, start sending {:?}", &buffer[0..hello_from_host.len()],hello_from_guest.as_bytes());
    socket.send(host_address,guest_port,hello_from_guest.as_bytes()).unwrap();

    socket.shutdown(host_address,guest_port).unwrap();
    let event = socket.wait_for_event().unwrap(); // wait for rst/shutdown

    debug!("The final event: {:?}",event);
    Ok(())

}

// pub fn probe_virtio_socket() -> Result<SocketDevice, ComponentInitError> {
//     let socket_devices = {
//         let virtio = VIRTIO_COMPONENT.get().unwrap();
//         virtio.get_device(VirtioDeviceType::Socket)
//     };

//     for device in socket_devices {
//         let device = if let VirtioDevice::Socket(socket_device) =
//             device.device
//         {
//             socket_device
//         } else {
//             panic!("Invalid device type")
//         };
//         // FIXME: deal with multiple socket devices
//         return Ok(device);
//     }

//     Err(ComponentInitError::Unknown)
// }
