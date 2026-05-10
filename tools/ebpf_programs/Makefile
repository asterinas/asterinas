CLANG ?= clang
OBJCOPY ?= llvm-objcopy
CFLAGS = -O2 -target bpf

PROGS = accept drop check_meta

all: $(PROGS:%=%.bin)

%.o: %.c
	$(CLANG) $(CFLAGS) -c $< -o $@

%.bin: %.o
	$(OBJCOPY) -O binary $< $@

clean:
	rm -f *.o *.bin

.PHONY: all clean
