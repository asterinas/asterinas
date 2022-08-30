.PHONY: build clean run
build: hello_world.s
	@gcc -static -nostdlib hello_world.s -o hello_world
clean:
	@rm hello_world
run: build
	@./hello_world