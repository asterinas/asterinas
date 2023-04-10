.PHONY: build clean run

build: hello.c
	@gcc -static -mno-sse hello.c -o hello

clean:
	@rm hello

run: build
	@./hello