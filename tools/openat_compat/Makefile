CC = gcc
CFLAGS = -O2 -Wall

all: ultimate_test_gen

ultimate_test_gen: ultimate_test_gen.c
  $(CC) $(CFLAGS) -o $@ $

clean:
	rm -f ultimate_test_gen

.PHONY: all clean
