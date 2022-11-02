.PHONY: build clean run
build: divide_zero.c sig_procmask.c sig_action.c
	@gcc -static divide_zero.c -o divide_zero
	@gcc -static sig_procmask.c -o sig_procmask
	@gcc -static sig_action.c -o sig_action
clean:
	@rm divide_zero sig_procmask sig_action
run: build
	@./sig_procmask
	@./sig_action
