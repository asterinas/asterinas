// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"

struct expression {
	struct sock_filter code[8];
	unsigned short len;
	unsigned expected;
};

static void check_expression(const struct expression *expr)
{
	struct sock_filter code[14] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS, 0),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_getppid, 1, 0),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	memcpy(code + 3, expr->code, expr->len * sizeof(code[0]));
	code[3 + expr->len] =
		(struct sock_filter)BPF_STMT(BPF_ALU | BPF_AND | BPF_K, 0xffff);
	code[4 + expr->len] = (struct sock_filter)BPF_STMT(
		BPF_ALU | BPF_OR | BPF_K, SECCOMP_RET_ERRNO);
	code[5 + expr->len] = (struct sock_filter)BPF_STMT(BPF_RET | BPF_A, 0);
	CHECK(install(code, 6 + expr->len));
	CHECK_WITH(syscall(SYS_getppid),
		   expr->expected == 0 ? _ret == 0 :
					 _ret == -1 && errno == expr->expected);
}

static void arithmetic_and_registers(void)
{
	no_new_privs();
	const struct expression expressions[] = {
		{ { BPF_STMT(BPF_LD | BPF_IMM, UINT32_MAX),
		    BPF_STMT(BPF_ALU | BPF_ADD | BPF_K, 3) },
		  2,
		  2 },
		{ { BPF_STMT(BPF_LD | BPF_IMM, 0),
		    BPF_STMT(BPF_ALU | BPF_SUB | BPF_K, UINT32_MAX) },
		  2,
		  1 },
		{ { BPF_STMT(BPF_LD | BPF_IMM, 0x80000001),
		    BPF_STMT(BPF_ALU | BPF_MUL | BPF_K, 2) },
		  2,
		  2 },
		{ { BPF_STMT(BPF_LD | BPF_IMM, 31),
		    BPF_STMT(BPF_ALU | BPF_DIV | BPF_K, 3) },
		  2,
		  10 },
		{ { BPF_STMT(BPF_LD | BPF_IMM, 0xf3),
		    BPF_STMT(BPF_ALU | BPF_AND | BPF_K, 0x3f),
		    BPF_STMT(BPF_ALU | BPF_OR | BPF_K, 0x04),
		    BPF_STMT(BPF_ALU | BPF_XOR | BPF_K, 0x07) },
		  4,
		  0x30 },
		{ { BPF_STMT(BPF_LD | BPF_IMM, 3),
		    BPF_STMT(BPF_ALU | BPF_LSH | BPF_K, 5),
		    BPF_STMT(BPF_ALU | BPF_RSH | BPF_K, 3) },
		  3,
		  12 },
		{ { BPF_STMT(BPF_LD | BPF_IMM, UINT32_MAX),
		    BPF_STMT(BPF_ALU | BPF_NEG, 0) },
		  2,
		  1 },
		{ { BPF_STMT(BPF_LDX | BPF_IMM, 3),
		    BPF_STMT(BPF_LD | BPF_IMM, 10),
		    BPF_STMT(BPF_ALU | BPF_ADD | BPF_X, 0),
		    BPF_STMT(BPF_ALU | BPF_MUL | BPF_X, 0),
		    BPF_STMT(BPF_ALU | BPF_SUB | BPF_X, 0),
		    BPF_STMT(BPF_ALU | BPF_DIV | BPF_X, 0) },
		  6,
		  12 },
		{ { BPF_STMT(BPF_LDX | BPF_IMM, 3),
		    BPF_STMT(BPF_LD | BPF_IMM, 5),
		    BPF_STMT(BPF_ALU | BPF_OR | BPF_X, 0),
		    BPF_STMT(BPF_ALU | BPF_XOR | BPF_X, 0),
		    BPF_STMT(BPF_ALU | BPF_AND | BPF_X, 0) },
		  5,
		  0 },
		{ { BPF_STMT(BPF_LDX | BPF_IMM, 33),
		    BPF_STMT(BPF_LD | BPF_IMM, 7),
		    BPF_STMT(BPF_ALU | BPF_LSH | BPF_X, 0) },
		  3,
		  14 },
		{ { BPF_STMT(BPF_LDX | BPF_IMM, 33),
		    BPF_STMT(BPF_LD | BPF_IMM, 7),
		    BPF_STMT(BPF_ALU | BPF_RSH | BPF_X, 0) },
		  3,
		  3 },
		{ { BPF_STMT(BPF_LD | BPF_IMM, 23),
		    BPF_STMT(BPF_MISC | BPF_TAX, 0), BPF_STMT(BPF_STX, 15),
		    BPF_STMT(BPF_LD | BPF_IMM, 0),
		    BPF_STMT(BPF_LDX | BPF_MEM, 15),
		    BPF_STMT(BPF_MISC | BPF_TXA, 0) },
		  6,
		  23 },
	};
	for (size_t i = 0; i < sizeof(expressions) / sizeof(expressions[0]);
	     ++i)
		check_expression(&expressions[i]);
}

static void conditional_jumps(void)
{
	no_new_privs();
	const unsigned short comparisons[] = { BPF_JEQ, BPF_JGT, BPF_JGE,
					       BPF_JSET };
	for (size_t i = 0; i < 4; ++i) {
		for (int use_x = 0; use_x < 2; ++use_x) {
			struct expression expr = {
				.code = {
					BPF_STMT(BPF_LDX | BPF_IMM, 3),
					BPF_STMT(BPF_LD | BPF_IMM, i == 0 ? 3 : 7),
					BPF_JUMP(BPF_JMP | comparisons[i] |
							 (use_x ? BPF_X : BPF_K),
						 3, 1, 0),
					BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
					BPF_STMT(BPF_JMP | BPF_JA, 1),
					BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS),
					BPF_STMT(BPF_LD | BPF_IMM, 17),
				},
				.len = 7,
				.expected = 17,
			};
			check_expression(&expr);
		}
	}
}

static void dynamic_division_by_zero(void)
{
	no_new_privs();
	struct sock_filter code[] = {
		BPF_STMT(BPF_LD | BPF_W | BPF_ABS, 0),
		BPF_JUMP(BPF_JMP | BPF_JEQ | BPF_K, SYS_getppid, 1, 0),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
		BPF_STMT(BPF_LDX | BPF_IMM, 0),
		BPF_STMT(BPF_ALU | BPF_DIV | BPF_X, 0),
		BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW),
	};
	CHECK(install(code, 6));
	syscall(SYS_getppid);
	_exit(1);
}

FN_TEST(interpreter)
{
	TEST_RES(run_child(arithmetic_and_registers, 0), _ret == 0);
	TEST_RES(run_child(conditional_jumps, 0), _ret == 0);
	TEST_RES(run_child(dynamic_division_by_zero, SIGSYS), _ret == 0);
}
END_TEST()
