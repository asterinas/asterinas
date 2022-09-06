.PHONY: build clean run
build: hello_world.s
	@gcc -static -nostdlib fork.s -o fork
clean:
	@rm fork
run: build
	@./fork