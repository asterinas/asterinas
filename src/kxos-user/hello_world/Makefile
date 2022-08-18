.PHONY: build clean run
build: hello_world.s
	@nasm -f elf64 hello_world.s -o hello_world.o
	@ld hello_world.o -o hello_world
	@rm hello_world.o
clean:
	@rm hello_world
run: build
	@./hello_world
