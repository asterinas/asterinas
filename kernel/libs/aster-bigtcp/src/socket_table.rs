// SPDX-License-Identifier: MPL-2.0

//! This module defines the socket table, which manages all TCP and UDP sockets,
//! for efficiently inserting, looking up, and removing sockets.

use alloc::{boxed::Box, sync::Arc, vec::Vec};
use core::net::Ipv4Addr;

use jhash::{jhash_1vals, jhash_3vals};
use ostd::const_assert;
use smoltcp::wire::{IpAddress, IpEndpoint, IpListenEndpoint};

use crate::{
    ext::Ext,
    socket::{TcpConnectionBg, TcpListenerBg, UdpSocketBg},
    wire::PortNum,
};

pub type SocketHash = u32;

/// A unique key for identifying a `TcpListener`.
///
/// Note that two `TcpListener`s cannot listen on the same address
/// even if both sockets set SO_REUSEADDR to true,
/// so there cannot be multiple listeners with the same `ListenerKey`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ListenerKey {
    addr: IpAddress,
    port: PortNum,
    hash: SocketHash,
}

impl ListenerKey {
    pub(crate) const fn new(addr: IpAddress, port: PortNum) -> Self {
        // FIXME: If the socket is listening on an unspecified address (0.0.0.0),
        // Linux will get the hash value by port only.
        let hash = hash_addr_port(addr, port);
        Self { addr, port, hash }
    }

    pub(crate) const fn hash(&self) -> SocketHash {
        self.hash
    }
}

impl From<IpListenEndpoint> for ListenerKey {
    fn from(listen_endpoint: IpListenEndpoint) -> Self {
        let addr = listen_endpoint
            .addr
            .unwrap_or(IpAddress::Ipv4(Ipv4Addr::UNSPECIFIED));
        let port = listen_endpoint.port;
        Self::new(addr, port)
    }
}

/// A unique key for identifying a `TcpConnection`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ConnectionKey {
    local_addr: IpAddress,
    local_port: PortNum,
    remote_addr: IpAddress,
    remote_port: PortNum,
    hash: SocketHash,
}

impl ConnectionKey {
    pub(crate) const fn new(
        local_addr: IpAddress,
        local_port: PortNum,
        remote_addr: IpAddress,
        remote_port: PortNum,
    ) -> Self {
        let hash = hash_local_remote(local_addr, local_port, remote_addr, remote_port);
        Self {
            local_addr,
            local_port,
            remote_addr,
            remote_port,
            hash,
        }
    }

    pub(crate) const fn hash(&self) -> SocketHash {
        self.hash
    }
}

impl From<(IpEndpoint, IpEndpoint)> for ConnectionKey {
    fn from(value: (IpEndpoint, IpEndpoint)) -> Self {
        Self::new(value.0.addr, value.0.port, value.1.addr, value.1.port)
    }
}

// FIXME: The following two constants should be randomly-generated at runtime
const HASH_SECRET: u32 = 0xdeadbeef;

// FIXME: This constant should be a per-net-namespace value
const NET_HASHMIX: u32 = 0xbeefdead;

const fn hash_local_remote(
    local_addr: IpAddress,
    local_port: PortNum,
    remote_addr: IpAddress,
    remote_port: PortNum,
) -> SocketHash {
    // FIXME: Deal with IPv6 addresses once IPv6 is supported.
    let IpAddress::Ipv4(local_ipv4) = local_addr;
    let IpAddress::Ipv4(remote_ipv4) = remote_addr;

    jhash_3vals(
        local_ipv4.to_bits(),
        remote_ipv4.to_bits(),
        (local_port as u32).wrapping_shl(16) | remote_port as u32,
        HASH_SECRET.wrapping_add(NET_HASHMIX),
    )
}

const fn hash_addr_port(addr: IpAddress, port: PortNum) -> SocketHash {
    // FIXME: Deal with IPv6 addresses once IPv6 is supported.
    let IpAddress::Ipv4(ipv4_addr) = addr;

    jhash_1vals(ipv4_addr.to_bits(), NET_HASHMIX) ^ (port as u32)
}

/// The socket table manages TCP and UDP sockets.
///
/// Unlike the Linux inet hashtable, which is shared across a single network namespace,
/// this table is currently limited to a single interface.
///
// TODO: Modify the table to be shared across a single network namespace
// to support INADDR_ANY (0.0.0.0).
pub(crate) struct SocketTable<E: Ext> {
    // TODO: Linux has two hashtables for listeners:
    // the first is hashed by local address and port,
    // the second is hashed by local port only.
    // The second table is the only place where sockets listening on INADDR_ANY (0.0.0.0) can exist.
    // Since we do not yet support INADDR_ANY, we only have the first table here.
    listener_buckets: Box<[ListenerHashBucket<E>]>,
    connection_buckets: Box<[ConnectionHashBucket<E>]>,
    // Linux does not include UDP sockets in the inet hashtable.
    // Here we include UDP sockets in the socket table for simplicity.
    // Note that multiple UDP sockets can be bound to the same address,
    // so we cannot use (addr, port) as a _unique_ key for UDP sockets.
    udp_sockets: Vec<Arc<UdpSocketBg<E>>>,
}

// On Linux, the number of buckets is determined at runtime based on the available memory.
// For simplicity, we use fixed values here.
// The bucket count should be a power of 2 to ensure efficient modulo calculations.
const LISTENER_BUCKET_COUNT: u32 = 64;
const LISTENER_BUCKET_MASK: u32 = LISTENER_BUCKET_COUNT - 1;
const CONNECTION_BUCKET_COUNT: u32 = 8192;
const CONNECTION_BUCKET_MASK: u32 = CONNECTION_BUCKET_COUNT - 1;

const_assert!(LISTENER_BUCKET_COUNT.is_power_of_two());
const_assert!(CONNECTION_BUCKET_COUNT.is_power_of_two());

impl<E: Ext> SocketTable<E> {
    pub(crate) fn new() -> Self {
        let listener_buckets = (0..LISTENER_BUCKET_COUNT)
            .map(|_| ListenerHashBucket::new())
            .collect();

        let connection_buckets = (0..CONNECTION_BUCKET_COUNT)
            .map(|_| ConnectionHashBucket::new())
            .collect();

        let udp_sockets = Vec::new();

        Self {
            listener_buckets,
            connection_buckets,
            udp_sockets,
        }
    }

    /// Inserts a TCP listener into the table.
    ///
    /// If a socket with the same [`ListenerKey`] has already been inserted,
    /// this method will return an error and the listener will not be inserted.
    pub(crate) fn insert_listener(
        &mut self,
        listener: Arc<TcpListenerBg<E>>,
    ) -> Result<(), Arc<TcpListenerBg<E>>> {
        let key = listener.listener_key();

        let bucket = {
            let hash = key.hash();
            let bucket_index = hash & LISTENER_BUCKET_MASK;
            &mut self.listener_buckets[bucket_index as usize]
        };

        if bucket
            .listeners
            .iter()
            .any(|tcp_listener| tcp_listener.listener_key() == listener.listener_key())
        {
            return Err(listener);
        }

        bucket.listeners.push(listener);
        Ok(())
    }

    pub(crate) fn insert_connection(
        &mut self,
        connection: Arc<TcpConnectionBg<E>>,
    ) -> Result<(), Arc<TcpConnectionBg<E>>> {
        let key = connection.connection_key();

        let bucket = {
            let hash = key.hash();
            let bucket_index = hash & CONNECTION_BUCKET_MASK;
            &mut self.connection_buckets[bucket_index as usize]
        };

        if bucket
            .connections
            .iter()
            .any(|tcp_connection| tcp_connection.connection_key() == connection.connection_key())
        {
            return Err(connection);
        }

        bucket.connections.push(connection);
        Ok(())
    }

    pub(crate) fn insert_udp_socket(&mut self, udp_socket: Arc<UdpSocketBg<E>>) {
        debug_assert!(!self
            .udp_sockets
            .iter()
            .any(|socket| Arc::ptr_eq(socket, &udp_socket)));
        self.udp_sockets.push(udp_socket);
    }

    pub(crate) fn lookup_listener(&self, key: &ListenerKey) -> Option<&Arc<TcpListenerBg<E>>> {
        let bucket = {
            let hash = key.hash();
            let bucket_index = hash & LISTENER_BUCKET_MASK;
            &self.listener_buckets[bucket_index as usize]
        };

        bucket
            .listeners
            .iter()
            .find(|listener| listener.listener_key() == key)
    }

    pub(crate) fn lookup_connection(
        &self,
        key: &ConnectionKey,
    ) -> Option<&Arc<TcpConnectionBg<E>>> {
        let bucket = {
            let hash = key.hash();
            let bucket_index = hash & CONNECTION_BUCKET_MASK;
            &self.connection_buckets[bucket_index as usize]
        };

        bucket
            .connections
            .iter()
            .find(|connection| connection.connection_key() == key)
    }

    pub(crate) fn remove_listener(&mut self, key: &ListenerKey) -> Option<Arc<TcpListenerBg<E>>> {
        let bucket = {
            let hash = key.hash();
            let bucket_index = hash & LISTENER_BUCKET_MASK;
            &mut self.listener_buckets[bucket_index as usize]
        };

        let index = bucket
            .listeners
            .iter()
            .position(|tcp_listener| tcp_listener.listener_key() == key)?;
        Some(bucket.listeners.swap_remove(index))
    }

    pub(crate) fn remove_dead_tcp_connection(&mut self, key: &ConnectionKey) {
        let bucket = {
            let hash = key.hash();
            let bucket_index = hash & CONNECTION_BUCKET_MASK;
            &mut self.connection_buckets[bucket_index as usize]
        };

        let index = bucket
            .connections
            .iter()
            .position(|tcp_connection| tcp_connection.connection_key() == key)
            .unwrap();
        let connection = bucket.connections.swap_remove(index);
        debug_assert!(
            !connection.poll_key().is_active(),
            "there should be no need to poll a dead TCP connection",
        );

        connection.notify_dead_events();
    }

    pub(crate) fn remove_udp_socket(
        &mut self,
        socket: &Arc<UdpSocketBg<E>>,
    ) -> Option<Arc<UdpSocketBg<E>>> {
        let index = self
            .udp_sockets
            .iter()
            .position(|udp_socket| Arc::ptr_eq(udp_socket, socket))?;
        Some(self.udp_sockets.swap_remove(index))
    }

    pub(crate) fn udp_socket_iter(&self) -> impl Iterator<Item = &Arc<UdpSocketBg<E>>> {
        self.udp_sockets.iter()
    }
}

impl<E: Ext> Default for SocketTable<E> {
    fn default() -> Self {
        Self::new()
    }
}

struct ListenerHashBucket<E: Ext> {
    listeners: Vec<Arc<TcpListenerBg<E>>>,
}

impl<E: Ext> ListenerHashBucket<E> {
    const fn new() -> Self {
        Self {
            listeners: Vec::new(),
        }
    }
}

struct ConnectionHashBucket<E: Ext> {
    connections: Vec<Arc<TcpConnectionBg<E>>>,
}

impl<E: Ext> ConnectionHashBucket<E> {
    const fn new() -> Self {
        Self {
            connections: Vec::new(),
        }
    }
}
