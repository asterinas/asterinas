.PHONY: build clean run
build: signal_test.c
	@gcc -static signal_test.c -o signal_test
clean:
	@rm signal_test
run: build
	@./signal_test
