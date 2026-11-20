// SPDX-License-Identifier: MPL-2.0

#include "seccomp_common.h"
#include <seccomp.h>
#include <sys/stat.h>

/* Every case also exports a named fixture, so identical BPF can be replayed. */
static int failures;
static unsigned int cases;
static unsigned int arg_index;
static enum scmp_compare comparison;
static uint64_t threshold;
static int use_cmp32;
static const char *case_name;

static scmp_filter_ctx context(uint32_t action, unsigned int optimize)
{
	scmp_filter_ctx ctx = CHECK_WITH(seccomp_init(action), _ret != NULL);
	CHECK(seccomp_attr_set(ctx, SCMP_FLTATR_CTL_OPTIMIZE, optimize));
	return ctx;
}

static void load_fixture(const char *directory, const char *name)
{
	char path[512];
	CHECK_WITH(snprintf(path, sizeof(path), "%s/%s.bpf", directory, name),
		   _ret > 0 && _ret < sizeof(path));
	int fd = CHECK(open(path, O_RDONLY));
	struct stat stat;
	CHECK(fstat(fd, &stat));
	CHECK_WITH(stat.st_size,
		   _ret > 0 && _ret % sizeof(struct sock_filter) == 0 &&
			   _ret / sizeof(struct sock_filter) <= 4096);
	struct sock_filter *code =
		CHECK_WITH(malloc(stat.st_size), _ret != NULL);
	CHECK_WITH(read(fd, code, stat.st_size), _ret == stat.st_size);
	CHECK(close(fd));
	no_new_privs();
	CHECK(install(code, stat.st_size / sizeof(*code)));
	free(code);
}

static void verify_generated(scmp_filter_ctx ctx, const char *directory,
			     const char *name)
{
	char path[512];
	CHECK_WITH(snprintf(path, sizeof(path), "%s/%s.bpf", directory, name),
		   _ret > 0 && _ret < sizeof(path));
	int reference = CHECK(open(path, O_RDONLY));
	FILE *generated = CHECK_WITH(tmpfile(), _ret != NULL);
	CHECK(seccomp_export_bpf(ctx, fileno(generated)));
	struct stat left, right;
	CHECK(fstat(reference, &left));
	CHECK(fstat(fileno(generated), &right));
	CHECK_WITH(left.st_size, _ret > 0 && _ret == right.st_size);
	CHECK(lseek(fileno(generated), 0, SEEK_SET));
	unsigned char *a = CHECK_WITH(malloc(left.st_size), _ret != NULL);
	unsigned char *b = CHECK_WITH(malloc(left.st_size), _ret != NULL);
	CHECK_WITH(read(reference, a, left.st_size), _ret == left.st_size);
	CHECK_WITH(read(fileno(generated), b, left.st_size),
		   _ret == left.st_size);
	CHECK_WITH(memcmp(a, b, left.st_size), _ret == 0);
	printf("LIBSECCOMP_BPF name=%s bytes=%ld matched=1\n", name,
	       (long)left.st_size);
	free(a);
	free(b);
	CHECK(close(reference));
	CHECK(fclose(generated));
}

static void exercise(scmp_filter_ctx ctx, const char *name, void (*probe)(void),
		     int expected_signal)
{
	const char *directory = getenv("SECCOMP_EXPORT_DIR");
	if (directory) {
		char path[512];
		CHECK_WITH(snprintf(path, sizeof(path), "%s/%s.bpf", directory,
				    name),
			   _ret > 0 && _ret < sizeof(path));
		int fd = CHECK(open(path, O_WRONLY | O_CREAT | O_EXCL, 0600));
		CHECK(seccomp_export_bpf(ctx, fd));
		CHECK(close(fd));
		seccomp_release(ctx);
		++cases;
		return;
	}
	directory = getenv("SECCOMP_VERIFY_DIR");
	if (directory && !getenv("SECCOMP_BPF_DIR"))
		verify_generated(ctx, directory, name);
	fflush(NULL);
	pid_t child = CHECK(fork());
	if (!child) {
		alarm(15);
		directory = getenv("SECCOMP_BPF_DIR");
		if (directory)
			load_fixture(directory, name);
		else
			CHECK(seccomp_load(ctx));
		seccomp_release(ctx);
		case_name = name;
		probe();
		fflush(stdout);
		_exit(0);
	}
	seccomp_release(ctx);
	int status;
	CHECK_WITH(waitpid(child, &status, 0), _ret == child);
	int ok = expected_signal ?
			 (WIFSIGNALED(status) &&
			  WTERMSIG(status) == expected_signal) :
			 (WIFEXITED(status) && WEXITSTATUS(status) == 0);
	printf("LIBSECCOMP_CASE name=%s mode=%s status=%#x expected_signal=%d ok=%d\n",
	       name, getenv("SECCOMP_BPF_DIR") ? "fixed" : "library", status,
	       expected_signal, ok);
	++cases;
	failures += !ok;
}

static int matches(uint64_t value)
{
	switch (comparison) {
	case SCMP_CMP_EQ:
		return value == threshold;
	case SCMP_CMP_NE:
		return value != threshold;
	case SCMP_CMP_LT:
		return value < threshold;
	case SCMP_CMP_LE:
		return value <= threshold;
	case SCMP_CMP_GT:
		return value > threshold;
	case SCMP_CMP_GE:
		return value >= threshold;
	case SCMP_CMP_MASKED_EQ:
		return (value & UINT64_C(0xff00ff00ff00ff00)) ==
		       (threshold & UINT64_C(0xff00ff00ff00ff00));
	default:
		abort();
	}
}

static void comparison_probe(void)
{
	uint64_t samples[] = { 0,
			       1,
			       UINT32_MAX,
			       UINT64_C(0x100000000),
			       UINT64_C(0x8000000000000000),
			       UINT64_MAX,
			       threshold,
			       threshold - 1,
			       threshold + 1,
			       threshold ^ UINT64_C(0x0000000100000001) };
	unsigned int denied = 0;
	for (size_t i = 0; i < sizeof(samples) / sizeof(samples[0]); ++i) {
		uint64_t args[6] = { 11, 22, 33, 44, 55, 66 };
		args[arg_index] = samples[i];
		errno = 0;
		long result = syscall(SYS_getppid, args[0], args[1], args[2],
				      args[3], args[4], args[5]);
		int error = errno;
		int match = matches(samples[i]);
		if (result == -1 && error == EACCES)
			denied |= 1U << i;
		if (match ? !(result == -1 && error == EACCES) :
			    !(result > 0 && error == 0))
			fprintf(stderr,
				"comparison %s sample=%zu value=%#llx result=%ld errno=%d match=%d\n",
				case_name, i, (unsigned long long)samples[i],
				result, error, match);
		CHECK_WITH(result, match ? (_ret == -1 && error == EACCES) :
					   (_ret > 0 && error == 0));
	}
	printf("LIBSECCOMP_VECTOR name=%s denied=%#x probes=10\n", case_name,
	       denied);
}

static void comparisons(void)
{
	uint64_t bounds[] = { 0, UINT32_MAX, UINT64_C(0x100000000),
			      UINT64_C(0x8000000000000000), UINT64_MAX };
	for (unsigned int optimize = 1; optimize <= 2; ++optimize) {
		for (comparison = SCMP_CMP_NE; comparison <= SCMP_CMP_MASKED_EQ;
		     ++comparison) {
			for (arg_index = 0; arg_index < 6; ++arg_index) {
				for (size_t b = 0;
				     b < sizeof(bounds) / sizeof(bounds[0]);
				     ++b) {
					threshold = bounds[b];
					scmp_filter_ctx ctx = context(
						SCMP_ACT_ALLOW, optimize);
					struct scmp_arg_cmp cmp = SCMP_CMP64(
						arg_index, comparison,
						threshold);
					if (comparison == SCMP_CMP_MASKED_EQ) {
						cmp.datum_a = UINT64_C(
							0xff00ff00ff00ff00);
						cmp.datum_b = threshold &
							      cmp.datum_a;
					}
					CHECK(seccomp_rule_add_array(
						ctx, SCMP_ACT_ERRNO(EACCES),
						SCMP_SYS(getppid), 1, &cmp));
					char name[96];
					snprintf(name, sizeof(name),
						 "cmp-%u-%u-%u-%zu", optimize,
						 comparison, arg_index, b);
					exercise(ctx, name, comparison_probe,
						 0);
				}
			}
		}
	}
}

static void width_probe(void)
{
	uint64_t bound = use_cmp32 ? UINT32_MAX : UINT64_MAX;
	uint64_t values[] = { UINT32_MAX, UINT64_MAX,
			      UINT64_C(0xffffffff00000000), 0 };
	for (size_t i = 0; i < sizeof(values) / sizeof(values[0]); ++i) {
		uint64_t args[6] = { 0 };
		args[arg_index] = values[i];
		errno = 0;
		long result = syscall(SYS_getppid, args[0], args[1], args[2],
				      args[3], args[4], args[5]);
		int error = errno;
		CHECK_WITH(result, values[i] == bound ?
					   (_ret == -1 && error == EACCES) :
					   (_ret > 0 && error == 0));
	}
}

static void widths(void)
{
	/* CMP32 truncates the rule datum, not the syscall's 64-bit argument. */
	for (use_cmp32 = 0; use_cmp32 <= 1; ++use_cmp32) {
		for (arg_index = 0; arg_index < 6; ++arg_index) {
			scmp_filter_ctx ctx = context(SCMP_ACT_ALLOW, 1);
			struct scmp_arg_cmp cmp =
				use_cmp32 ?
					SCMP_CMP32(arg_index, SCMP_CMP_EQ, -1) :
					SCMP_CMP64(arg_index, SCMP_CMP_EQ,
						   UINT64_MAX);
			CHECK(seccomp_rule_add_exact_array(
				ctx, SCMP_ACT_ERRNO(EACCES), SCMP_SYS(getppid),
				1, &cmp));
			char name[64];
			snprintf(name, sizeof(name), "width-%d-%u", use_cmp32,
				 arg_index);
			exercise(ctx, name, width_probe, 0);
		}
	}
}

static void allowlist_probe(void)
{
	CHECK(syscall(SYS_getpid));
	CHECK_WITH(syscall(SYS_getppid), _ret == -1 && errno == EPERM);
	CHECK_WITH(syscall(SYS_getuid), _ret == -1 && errno == EPERM);
}

static void multiple_probe(void)
{
	CHECK_WITH(syscall(SYS_getppid, 10UL, 20UL, 30UL, 40UL, 50UL, 60UL),
		   _ret == -1 && errno == EBUSY);
	for (unsigned int i = 0; i < 6; ++i) {
		unsigned long args[] = { 10, 20, 30, 40, 50, 60 };
		++args[i];
		CHECK(syscall(SYS_getppid, args[0], args[1], args[2], args[3],
			      args[4], args[5]));
	}
	CHECK_WITH(syscall(SYS_getppid, 100UL, 200UL),
		   _ret == -1 && errno == EBUSY);
	CHECK_WITH(syscall(SYS_getppid, 100UL, 201UL),
		   _ret == -1 && errno == EBUSY);
	CHECK(syscall(SYS_getppid, 101UL, 200UL));
}

static void large_probe(void)
{
	for (unsigned long i = 0; i < 400; ++i) {
		CHECK_WITH(syscall(SYS_getppid, i * 2),
			   _ret == -1 && errno == EACCES);
		CHECK(syscall(SYS_getppid, i * 2 + 1));
	}
	/* These numbers are synthetic; matching ERRNO must precede dispatch. */
	for (int i = 0; i < 128; ++i)
		CHECK_WITH(syscall(10000 + i), _ret == -1 && errno == EBUSY);
	CHECK_WITH(syscall(9999), _ret == -1 && errno == ENOSYS);
	CHECK(syscall(SYS_getpid));
}

static void policy_shapes(void)
{
	for (unsigned int optimize = 1; optimize <= 2; ++optimize) {
		char name[64];
		scmp_filter_ctx ctx = context(SCMP_ACT_ERRNO(EPERM), optimize);
		int allowed[] = { SCMP_SYS(getpid),	SCMP_SYS(exit),
				  SCMP_SYS(exit_group), SCMP_SYS(write),
				  SCMP_SYS(close),	SCMP_SYS(munmap),
				  SCMP_SYS(brk),	SCMP_SYS(rt_sigreturn) };
		for (size_t i = 0; i < sizeof(allowed) / sizeof(allowed[0]);
		     ++i)
			CHECK(seccomp_rule_add(ctx, SCMP_ACT_ALLOW, allowed[i],
					       0));
		snprintf(name, sizeof(name), "allowlist-%u", optimize);
		exercise(ctx, name, allowlist_probe, 0);

		ctx = context(SCMP_ACT_ALLOW, optimize);
		struct scmp_arg_cmp cmp[6];
		for (unsigned int i = 0; i < 6; ++i)
			cmp[i] = SCMP_CMP64(i, SCMP_CMP_EQ, (i + 1) * 10);
		CHECK(seccomp_rule_add_array(ctx, SCMP_ACT_ERRNO(EBUSY),
					     SCMP_SYS(getppid), 6, cmp));
		CHECK(seccomp_rule_add(ctx, SCMP_ACT_ERRNO(EBUSY),
				       SCMP_SYS(getppid), 2,
				       SCMP_A0_64(SCMP_CMP_EQ, 100),
				       SCMP_A1_64(SCMP_CMP_EQ, 200)));
		/* Duplicate, then broader overlapping rule; both have the same action. */
		CHECK(seccomp_rule_add(ctx, SCMP_ACT_ERRNO(EBUSY),
				       SCMP_SYS(getppid), 2,
				       SCMP_A0_64(SCMP_CMP_EQ, 100),
				       SCMP_A1_64(SCMP_CMP_EQ, 200)));
		CHECK(seccomp_rule_add(ctx, SCMP_ACT_ERRNO(EBUSY),
				       SCMP_SYS(getppid), 1,
				       SCMP_A0_64(SCMP_CMP_EQ, 100)));
		snprintf(name, sizeof(name), "multiple-%u", optimize);
		exercise(ctx, name, multiple_probe, 0);

		ctx = context(SCMP_ACT_ALLOW, optimize);
		for (unsigned int i = 0; i < 400; ++i)
			CHECK(seccomp_rule_add(ctx, SCMP_ACT_ERRNO(EACCES),
					       SCMP_SYS(getppid), 1,
					       SCMP_A0_64(SCMP_CMP_EQ, i * 2)));
		for (unsigned int i = 0; i < 128; ++i) {
			CHECK(seccomp_rule_add_exact(ctx, SCMP_ACT_ERRNO(EBUSY),
						     10000 + i, 0));
			CHECK(seccomp_syscall_priority(ctx, 10000 + i,
						       255 - i));
		}
		CHECK(seccomp_syscall_priority(ctx, SCMP_SYS(getppid), 255));
		snprintf(name, sizeof(name), "large-%u", optimize);
		exercise(ctx, name, large_probe, 0);
	}
}

static void fatal_probe(void)
{
	syscall(SYS_getppid);
	_exit(91);
}

static void actions(void)
{
	uint32_t actions[] = { SCMP_ACT_KILL_THREAD, SCMP_ACT_KILL_PROCESS,
			       SCMP_ACT_TRAP };
	for (size_t i = 0; i < sizeof(actions) / sizeof(actions[0]); ++i) {
		scmp_filter_ctx ctx = context(SCMP_ACT_ALLOW, 1);
		CHECK(seccomp_rule_add(ctx, actions[i], SCMP_SYS(getppid), 0));
		char name[64];
		snprintf(name, sizeof(name), "fatal-%zu", i);
		exercise(ctx, name, fatal_probe, SIGSYS);
	}
}

FN_TEST(libseccomp_rules)
{
	const struct scmp_version *version = seccomp_version();
	printf("LIBSECCOMP_VERSION=%u.%u.%u API=%u ARCH=%#x\n", version->major,
	       version->minor, version->micro, seccomp_api_get(),
	       seccomp_arch_native());
	CHECK_WITH(seccomp_api_get(), _ret >= 3);
	comparisons();
	widths();
	policy_shapes();
	actions();
	printf("LIBSECCOMP_RULES_RESULT=%d cases=%u mode=%s\n",
	       failures ? 1 : 0, cases,
	       getenv("SECCOMP_EXPORT_DIR") ?
		       "export" :
		       (getenv("SECCOMP_BPF_DIR") ? "fixed" : "library"));
	TEST_RES(failures, _ret == 0);
}
END_TEST()
