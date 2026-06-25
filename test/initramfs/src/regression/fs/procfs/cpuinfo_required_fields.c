// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <string.h>

#include "../../common/test.h"

#define CPUINFO_BUF_SIZE 65536
#define ARRAY_SIZE(array) (sizeof(array) / sizeof((array)[0]))

#if defined(__x86_64__) || defined(__i386__)
static const char *required_fields[] = {
	"processor",	 "vendor_id",	     "cpu family",
	"model\t\t:",	 "model name",	     "stepping",
	"cpu MHz",	 "cache size",	     "physical id",
	"siblings",	 "core id",	     "cpu cores",
	"apicid\t\t:",	 "initial apicid",   "fpu\t\t:",
	"fpu_exception", "cpuid level",	     "wp",
	"bogomips",	 "clflush size",     "cache_alignment",
	"address sizes", "power management",
};
#elif defined(__riscv)
static const char *required_fields[] = { "processor", "hart" };
#else
static const char *required_fields[] = { "processor" };
#endif

FN_TEST(cpuinfo_required_fields)
{
	char cpuinfo[CPUINFO_BUF_SIZE];
	FILE *fp = TEST_SUCC(fopen("/proc/cpuinfo", "r"));
	size_t len = TEST_RES(fread(cpuinfo, 1, sizeof(cpuinfo) - 1, fp),
			      _ret > 0 && _ret < sizeof(cpuinfo));

	TEST_SUCC(fclose(fp));
	cpuinfo[len] = '\0';

	for (size_t i = 0; i < ARRAY_SIZE(required_fields); i++) {
		TEST_RES(strstr(cpuinfo, required_fields[i]) != NULL,
			 _ret != 0);
	}
}
END_TEST()
