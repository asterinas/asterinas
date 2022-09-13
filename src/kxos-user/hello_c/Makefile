.PHONY: build clean run
build: hello.c
	@gcc -static hello.c -o hello
clean:
	@rm hello
run: build
	@./hello