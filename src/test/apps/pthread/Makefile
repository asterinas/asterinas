.PHONY: build clean run
build: pthread_test.c
	@gcc -static pthread_test.c -lpthread -o pthread_test
clean:
	@rm pthread_test
run: build
	@./pthread_test