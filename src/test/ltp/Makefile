LTP_SRC := ./ltp
LTP_TEST := ./ltp_test
SYSCALL_JINUX := ./syscall-jinux
SKIP_LIST := ./skip-list

.PHONY: all clean
all: $(SYSCALL_JINUX) $(SKIP_LIST)
	./download_and_build_ltp.sh
	./copy_tests.sh
clean:
	rm -rf ./ltp
	rm -rf ./ltp_test
