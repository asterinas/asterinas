.PHONY: build clean run
build: hello.c execve.c
	@gcc -static hello.c -o hello
	@gcc -static execve.c -o execve
clean:
	@rm hello
	@rm execve
run: build
	@./execve