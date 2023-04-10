.PHONY: build clean run

build: fork.s
	@gcc -static -nostdlib fork.s -o fork

clean:
	@rm fork

run: build
	@./fork