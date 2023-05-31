.PHONY: build clean

build: tcp/client.c tcp/server.c udp/client.c udp/server.c
	@gcc tcp/client.c -o tcp/client
	@gcc tcp/server.c -o tcp/server
	@gcc udp/server.c -o udp/server
	@gcc udp/client.c -o udp/client

clean:
	@rm tcp/client
	@rm tcp/server
	@rm udp/client
	@rm udp/server
