.PHONY: build clean run

build: fork.c
	@gcc -static fork.c -o fork

clean:
	@rm fork

run: build
	@./fork