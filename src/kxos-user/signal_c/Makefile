.PHONY: build clean run
build: divide_zero.c sig_procmask.c
	@gcc -static divide_zero.c -o divide_zero
	@gcc -static sig_procmask.c -o sig_procmask
clean:
	@rm divide_zero sig_procmask
run: build
	@./sig_procmask
	@./divide_zero
