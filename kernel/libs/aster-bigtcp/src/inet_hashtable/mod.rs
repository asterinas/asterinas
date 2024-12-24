// SPDX-License-Identifier: MPL-2.0

mod jhash;

use alloc::{boxed::Box, collections::btree_map::BTreeMap, sync::Arc};
use core::net::Ipv4Addr;

pub use jhash::{jhash_1vals, jhash_2vals, jhash_3vals, jhash_array};
use smoltcp::wire::{IpAddress, IpEndpoint, IpListenEndpoint};

use crate::{
    ext::Ext,
    socket::{TcpConnectionBg, TcpListenerBg, UdpSocketBg},
    wire::PortNum,
};

pub type SocketHash = u32;
pub(crate) type UdpSocketKey = ListenerKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ListenerKey {
    addr: IpAddress,
    port: PortNum,
}

impl ListenerKey {
    pub(crate) fn calc_hash(&self) -> SocketHash {
        // FIXME: If the socket is listening at unspecified address(0.0.0.0),
        // Linux will get hash value by  port only.
        hash_addr_port(self.addr, self.port)
    }
}

impl From<(IpAddress, PortNum)> for ListenerKey {
    fn from(value: (IpAddress, PortNum)) -> Self {
        Self {
            addr: value.0,
            port: value.1,
        }
    }
}

impl From<IpListenEndpoint> for ListenerKey {
    fn from(listen_endpoint: IpListenEndpoint) -> Self {
        let addr = listen_endpoint
            .addr
            .unwrap_or(IpAddress::Ipv4(Ipv4Addr::UNSPECIFIED));
        let port = listen_endpoint.port;
        Self { addr, port }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ConnectionKey {
    local_addr: IpAddress,
    local_port: PortNum,
    remote_addr: IpAddress,
    remote_port: PortNum,
}

impl From<(IpAddress, PortNum, IpAddress, PortNum)> for ConnectionKey {
    fn from(value: (IpAddress, PortNum, IpAddress, PortNum)) -> Self {
        Self {
            local_addr: value.0,
            local_port: value.1,
            remote_addr: value.2,
            remote_port: value.3,
        }
    }
}

impl From<(IpEndpoint, IpEndpoint)> for ConnectionKey {
    fn from(value: (IpEndpoint, IpEndpoint)) -> Self {
        Self {
            local_addr: value.0.addr,
            local_port: value.0.port,
            remote_addr: value.1.addr,
            remote_port: value.1.port,
        }
    }
}

impl ConnectionKey {
    pub(crate) fn calc_hash(&self) -> SocketHash {
        hash_local_remote(
            self.local_addr,
            self.local_port,
            self.remote_addr,
            self.remote_port,
        )
    }
}

// FIXME: The following two constants should be randomly-generated at runtime
const HASH_SECRET: u32 = 0xdeadbeef;

// FIXME: This constant should be a per-namespace value
const NET_HASHMIX: u32 = 0xbeefdead;

fn hash_local_remote(
    local_addr: IpAddress,
    local_port: PortNum,
    remote_addr: IpAddress,
    remote_port: PortNum,
) -> SocketHash {
    // FIXME: Deal with IPv6 address once IPv6 is supported.
    let IpAddress::Ipv4(local_ipv4) = local_addr;
    let IpAddress::Ipv4(remote_ipv4) = remote_addr;

    jhash_3vals(
        local_ipv4.to_bits(),
        remote_ipv4.to_bits(),
        (local_port as u32).wrapping_shl(16) | remote_port as u32,
        HASH_SECRET.wrapping_add(NET_HASHMIX),
    )
}

fn hash_addr_port(addr: IpAddress, port: PortNum) -> SocketHash {
    // FIXME: Deal with IPv6 address once IPv6 is supported.
    let IpAddress::Ipv4(ipv4_addr) = addr;

    jhash_1vals(ipv4_addr.to_bits(), NET_HASHMIX) ^ (port as u32)
}

pub struct SocketTable<E: Ext> {
    // TODO: Linux has two hashtable for listeners.
    // the first is hashed by local port address and port,
    // the second is hashed by local port only.
    // The second is the only table well sockets listening in INADDR_ANY(0.0.0.0) can exist.
    // Since we haven't supported INADDR_ANY yet, we only have the first table here.
    listener_buckets: Box<[ListenerHashBucket<E>]>,
    connection_buckets: Box<[ConnectionHashBucket<E>]>,
    // Linux seems not to include udp sockets in inet hashtable.
    // Here we include udp sockets for simplicity.
    udp_buckets: Box<[UdpSocketHashBucket<E>]>,
}

impl<E: Ext> SocketTable<E> {
    // TODO: Use the same bucket counts as Linux
    const LISTEN_BUCKET_COUNT: u32 = 64;
    const CONNECTION_BUCKET_COUNT: u32 = 8192;
    const UDP_SOCKET_BUCKET_COUNT: u32 = 64;

    pub fn new() -> Self {
        let listen_buckets = (0..Self::LISTEN_BUCKET_COUNT)
            .map(|_| ListenerHashBucket::new())
            .collect();

        let connection_buckets = (0..Self::CONNECTION_BUCKET_COUNT)
            .map(|_| ConnectionHashBucket::new())
            .collect();

        let udp_buckets = (0..Self::UDP_SOCKET_BUCKET_COUNT)
            .map(|_| UdpSocketHashBucket::new())
            .collect();

        Self {
            listener_buckets: listen_buckets,
            connection_buckets,
            udp_buckets,
        }
    }

    pub fn insert_listener(&mut self, listener: Arc<TcpListenerBg<E>>) -> bool {
        let bucket = {
            let hash = listener.listener_hash();
            let bucket_index = hash % Self::LISTEN_BUCKET_COUNT;
            self.listener_buckets
                .get_mut(bucket_index as usize)
                .unwrap()
        };

        bucket.count += 1;

        let key = listener.listener_key();
        bucket.listeners.insert(key, listener).is_none()
    }

    pub fn insert_connection(&mut self, connection: Arc<TcpConnectionBg<E>>) -> bool {
        let bucket = {
            let hash = connection.connection_hash();
            let bucket_index = hash % Self::CONNECTION_BUCKET_COUNT;
            self.connection_buckets
                .get_mut(bucket_index as usize)
                .unwrap()
        };

        bucket.count += 1;

        let key = connection.connection_key();
        bucket.connections.insert(key, connection).is_none()
    }

    pub(crate) fn insert_udp_socket(&mut self, udp_socket: Arc<UdpSocketBg<E>>) -> bool {
        let bucket = {
            let hash = udp_socket.udp_socket_hash();
            let bucket_index = hash % Self::UDP_SOCKET_BUCKET_COUNT;
            self.udp_buckets.get_mut(bucket_index as usize).unwrap()
        };

        bucket.count += 1;

        let key = udp_socket.udp_socket_key();
        bucket.udp_sockets.insert(key, udp_socket).is_none()
    }

    pub(crate) fn lookup_listener(&self, key: &ListenerKey) -> Option<&Arc<TcpListenerBg<E>>> {
        let hash = key.calc_hash();

        let bucket = {
            let bucket_index = hash % Self::LISTEN_BUCKET_COUNT;
            self.listener_buckets.get(bucket_index as usize).unwrap()
        };

        bucket.listeners.get(key)
    }

    pub(crate) fn lookup_connection(
        &self,
        key: &ConnectionKey,
    ) -> Option<&Arc<TcpConnectionBg<E>>> {
        let hash = key.calc_hash();

        let bucket = {
            let bucket_index = hash % Self::CONNECTION_BUCKET_COUNT;
            self.connection_buckets.get(bucket_index as usize).unwrap()
        };

        bucket.connections.get(key)
    }

    pub(crate) fn remove_listener(
        &mut self,
        listener: &TcpListenerBg<E>,
    ) -> Option<Arc<TcpListenerBg<E>>> {
        let key = listener.listener_key();
        let hash = listener.listener_hash();
        debug_assert_eq!(key.calc_hash(), hash);

        let bucket = {
            let bucket_index = hash % Self::LISTEN_BUCKET_COUNT;
            self.listener_buckets
                .get_mut(bucket_index as usize)
                .unwrap()
        };

        if bucket.count == 0 {
            return None;
        }

        bucket.count -= 1;

        bucket.listeners.remove(&key)
    }

    pub(crate) fn remove_udp_socket(
        &mut self,
        socket: &UdpSocketBg<E>,
    ) -> Option<Arc<UdpSocketBg<E>>> {
        let key = socket.udp_socket_key();
        let hash = socket.udp_socket_hash();
        debug_assert_eq!(key.calc_hash(), hash);

        let bucket = {
            let bucket_index = hash % Self::UDP_SOCKET_BUCKET_COUNT;
            self.udp_buckets.get_mut(bucket_index as usize).unwrap()
        };

        if bucket.count == 0 {
            return None;
        }

        bucket.count -= 1;

        bucket.udp_sockets.remove(&key)
    }

    pub(crate) fn remove_dead_tcp_connections(&mut self) {
        for connection_bucket in self.connection_buckets.iter_mut() {
            for (_, tcp_conn) in connection_bucket
                .connections
                .extract_if(|_, connection| connection.is_dead())
            {
                tcp_conn.on_dead_events();
            }
        }
    }

    pub(crate) fn on_each_socket_events(&self) {
        for socket in self.tcp_listener_iter() {
            if socket.has_events() {
                socket.on_events();
            }
        }

        for socket in self.tcp_conn_iter() {
            if socket.has_events() {
                socket.on_events();
            }
        }

        for socket in self.udp_socket_iter() {
            if socket.has_events() {
                socket.on_events();
            }
        }
    }

    fn tcp_listener_iter(&self) -> impl Iterator<Item = &Arc<TcpListenerBg<E>>> {
        self.listener_buckets
            .iter()
            .flat_map(|bucket| bucket.listeners.values())
    }

    pub(crate) fn tcp_conn_iter(&self) -> impl Iterator<Item = &Arc<TcpConnectionBg<E>>> {
        self.connection_buckets
            .iter()
            .flat_map(|bucket| bucket.connections.values())
    }

    pub(crate) fn udp_socket_iter(&self) -> impl Iterator<Item = &Arc<UdpSocketBg<E>>> {
        self.udp_buckets
            .iter()
            .flat_map(|bucket| bucket.udp_sockets.values())
    }
}

impl<E: Ext> Default for SocketTable<E> {
    fn default() -> Self {
        Self::new()
    }
}

struct ListenerHashBucket<E: Ext> {
    count: usize,
    listeners: BTreeMap<ListenerKey, Arc<TcpListenerBg<E>>>,
}

impl<E: Ext> ListenerHashBucket<E> {
    const fn new() -> Self {
        Self {
            count: 0,
            listeners: BTreeMap::new(),
        }
    }
}

struct ConnectionHashBucket<E: Ext> {
    count: usize,
    connections: BTreeMap<ConnectionKey, Arc<TcpConnectionBg<E>>>,
}

impl<E: Ext> ConnectionHashBucket<E> {
    const fn new() -> Self {
        Self {
            count: 0,
            connections: BTreeMap::new(),
        }
    }
}

struct UdpSocketHashBucket<E: Ext> {
    count: usize,
    udp_sockets: BTreeMap<UdpSocketKey, Arc<UdpSocketBg<E>>>,
}

impl<E: Ext> UdpSocketHashBucket<E> {
    const fn new() -> Self {
        Self {
            count: 0,
            udp_sockets: BTreeMap::new(),
        }
    }
}
