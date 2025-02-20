#include <net/if.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

int main(int argc, char *argv[])
{
	struct if_nameindex *if_ni, *i;

	if_ni = if_nameindex();
	if (if_ni == NULL) {
		perror("if_nameindex");
		exit(EXIT_FAILURE);
	}

	for (i = if_ni; !(i->if_index == 0 && i->if_name == NULL); i++)
		printf("%u: %s\n", i->if_index, i->if_name);

	if_freenameindex(if_ni);

	exit(EXIT_SUCCESS);
}
