# Database Tools

## SQLite

[SQLite](https://www.sqlite.org/) is a C-language library that implements a small, fast, self-contained SQL database engine.

### Installation

```nix
environment.systemPackages = pkgs.sqlite;
```

### Verified Usage

#### Database operations

```bash
# Create new SQLite database and open it
sqlite3 database.db

# Execute SQL command directly
sqlite3 database.db "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT);"

# Insert data
sqlite3 database.db "INSERT INTO users (name) VALUES ('Alice');"

# Query data
sqlite3 database.db "SELECT * FROM users;"
```

## Redis

[Redis](https://redis.io/) is an in-memory data structure store used as a database, cache, and message broker.

### Installation

```nix
environment.systemPackages = pkgs.redis;
```

### Verified Usage

#### Key-value operations

```bash
# Start Redis server with specific configuration
redis-server --bind 10.0.2.15 --port 6379 --protected-mode no

# Connect to Redis server on specific host and port
redis-cli -h hostname -p 6379

# Set key-value pair
redis-cli SET mykey "Hello World"

# Get value by key
redis-cli GET mykey

# Delete key
redis-cli DEL mykey
```

## InfluxDB

[InfluxDB](https://influxdata.com/) is a time series database designed for high write and query loads.

### Installation

```nix
environment.systemPackages = pkgs.influxdb;
```

### Verified Usage

#### Time series operations

```bash
# Start with specific configuration file
influxd -config /etc/influxdb/influxdb.conf

# Connect to remote InfluxDB server
influx -host hostname -port 8086

# Create database
influx -execute "CREATE DATABASE mydb"

# Use database
influx -execute "USE mydb"

# Write data (InfluxDB line protocol)
influx -execute "INSERT cpu,host=server1 value=0.64"
influx -execute "INSERT memory,host=server1 used=80,total=100"

# Query data
influx -execute "SELECT * FROM cpu"
influx -execute "SELECT * FROM memory WHERE host='server1'"
```

## InfluxDB 3

[InfluxDB 3.x](https://github.com/influxdata/influxdb) is the next generation of InfluxDB with improved performance and features.

### Installation

```nix
environment.systemPackages = pkgs.influxdb3;
```

### Verified Usage

#### Time series operations

```bash
# Start InfluxDB 3.x server
influxdb3 serve --object-store file --data-dir ~/.influxdb3 --node-id testdb --http-bind 10.0.2.15:8086

# Connect to remote InfluxDB server
influx -host hostname -port 8086
```

## Etcd

[Etcd](https://etcd.io/) is a distributed key-value store that provides a reliable way to store data that needs to be accessed by a distributed system.

### Installation

```nix
environment.systemPackages = pkgs.etcd;
```

### Verified Usage

#### Distributed key-value operations

```bash
# Start etcd with specific listen addresses
etcd --listen-peer-urls=http://10.0.2.15:8081 --listen-client-urls=http://10.0.2.15:8080 --advertise-client-urls=http://10.0.2.15:8080

# Put key-value pair
etcdctl --endpoints=localhost:8080 put key1 value1

# Get value by key
etcdctl --endpoints=localhost:8080 get key1

# Delete key
etcdctl --endpoints=localhost:8080 del key1
```
