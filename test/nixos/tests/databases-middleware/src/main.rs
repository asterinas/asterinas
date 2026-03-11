// SPDX-License-Identifier: MPL-2.0

//! The test suite for databases and middleware applications on Asterinas NixOS.
//!
//! # Document maintenance
//!
//! An application's test suite and its "Verified Usage" section in Asterinas Book
//! should always be kept in sync.
//! So whenever you modify the test suite,
//! review the documentation and see if should be updated accordingly.

use nixos_test_framework::*;

nixos_test_main!();

// ============================================================================
// Relational Databases - SQLite
// ============================================================================

#[nixos_test]
fn sqlite_create_database(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(
        "sqlite3 /tmp/test.db \"CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);\"",
    )?;
    nixos_shell.run_cmd_and_expect("sqlite3 /tmp/test.db \".tables\"", "users")?;
    Ok(())
}

#[nixos_test]
fn sqlite_insert_query(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(
        "sqlite3 /tmp/test2.db \"CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);\"",
    )?;
    nixos_shell.run_cmd("sqlite3 /tmp/test2.db \"INSERT INTO users (name) VALUES ('Alice');\"")?;
    nixos_shell.run_cmd_and_expect("sqlite3 /tmp/test2.db \"SELECT * FROM users;\"", "Alice")?;
    Ok(())
}

#[nixos_test]
fn sqlite_select_query(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("sqlite3 /tmp/test3.db \"CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, value INTEGER);\"")?;
    nixos_shell.run_cmd(
        "sqlite3 /tmp/test3.db \"INSERT INTO items (name, value) VALUES ('item1', 100);\"",
    )?;
    nixos_shell.run_cmd(
        "sqlite3 /tmp/test3.db \"INSERT INTO items (name, value) VALUES ('item2', 200);\"",
    )?;
    nixos_shell.run_cmd_and_expect(
        "sqlite3 /tmp/test3.db \"SELECT name FROM items WHERE value > 150;\"",
        "item2",
    )?;
    Ok(())
}

// ============================================================================
// Key-Value Stores - Redis
// ============================================================================

#[nixos_test]
fn redis_set_get_del(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell
        .run_cmd("redis-server --bind 127.0.0.1 --port 6380 --protected-mode no --daemonize yes")?;
    nixos_shell.run_cmd("sleep 3")?;
    nixos_shell.run_cmd_and_expect("redis-cli -p 6380 ping", "PONG")?;
    nixos_shell.run_cmd("redis-cli -p 6380 SET mykey \"Hello World\"")?;
    nixos_shell.run_cmd_and_expect("redis-cli -p 6380 GET mykey", "Hello World")?;
    nixos_shell.run_cmd("redis-cli -p 6380 DEL mykey")?;
    nixos_shell.run_cmd_and_expect("redis-cli -p 6380 GET mykey", "nil")?;
    Ok(())
}

// ============================================================================
// Key-Value Stores - Valkey
// ============================================================================

#[nixos_test]
fn valkey_server(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd(
        "valkey-server --bind 127.0.0.1 --port 6381 --protected-mode no --daemonize yes",
    )?;
    nixos_shell.run_cmd("sleep 3")?;
    nixos_shell.run_cmd_and_expect("valkey-cli -p 6381 ping", "PONG")?;
    nixos_shell.run_cmd("valkey-cli -p 6381 set mykey \"Hello World\"")?;
    nixos_shell.run_cmd_and_expect("valkey-cli -p 6381 get mykey", "Hello World")?;
    nixos_shell.run_cmd("valkey-cli -p 6381 del mykey")?;
    nixos_shell.run_cmd_and_expect("valkey-cli -p 6381 get mykey", "nil")?;
    Ok(())
}

// ============================================================================
// Distributed Key-Value Stores - Etcd
// ============================================================================

#[nixos_test]
fn etcd_server(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("etcd --listen-peer-urls=http://127.0.0.1:2380 --listen-client-urls=http://127.0.0.1:2379 --advertise-client-urls=http://127.0.0.1:2379 &")?;
    nixos_shell.run_cmd("sleep 3")?;
    nixos_shell.run_cmd_and_expect(
        "etcdctl --endpoints=localhost:2379 endpoint health",
        "is healthy",
    )?;

    nixos_shell.run_cmd("etcdctl --endpoints=localhost:2379 put testkey testvalue")?;
    nixos_shell.run_cmd_and_expect(
        "etcdctl --endpoints=localhost:2379 get testkey",
        "testvalue",
    )?;
    nixos_shell.run_cmd_and_expect("etcdctl --endpoints=localhost:2379 del testkey", "1")?;
    nixos_shell.run_cmd_and_expect("etcdctl --endpoints=localhost:2379 get testkey", "")?;
    Ok(())
}

// ============================================================================
// Time Series Databases - InfluxDB
// ============================================================================

#[nixos_test]
fn influxdb_server(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("influxd config > /tmp/influxdb.conf")?;
    nixos_shell.run_cmd("sed -i '/bind-address/s/:8086/10.0.2.15:8086/' /tmp/influxdb.conf")?;
    nixos_shell.run_cmd("influxd -config /tmp/influxdb.conf > /tmp/influxd.log 2>&1 &")?;
    nixos_shell.run_cmd("sleep 10")?;

    nixos_shell.run_cmd("influx -host 10.0.2.15 -port 8086 -execute 'CREATE DATABASE testdb'")?;
    nixos_shell.run_cmd_and_expect(
        "influx -host 10.0.2.15 -port 8086 -execute 'USE testdb'",
        "Using database testdb",
    )?;
    nixos_shell.run_cmd("influx -host 10.0.2.15 -port 8086 -database testdb -execute 'INSERT cpu,host=server1 value=0.64'")?;
    nixos_shell.run_cmd_and_expect(
        "influx -host 10.0.2.15 -port 8086 -database testdb -execute 'SELECT * FROM cpu'",
        "server1",
    )?;
    Ok(())
}
