#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <string.h>
#include <sys/types.h>
#include <sys/socket.h>
#include <linux/netlink.h>

#define MAX_BUF_LEN 1024

int main()
{
	int sockfd;
	struct sockaddr_nl addr;
	socklen_t addrlen = sizeof(struct sockaddr_nl);

	sockfd = socket(AF_NETLINK, SOCK_DGRAM, NETLINK_USERSOCK);
	if (sockfd < 0) {
		perror("Failed to create socket");
		return -1;
	}

	// 绑定地址
	memset(&addr, 0, sizeof(struct sockaddr_nl));
	addr.nl_family = AF_NETLINK;
	addr.nl_groups = 1; // 不加入任何多播组

	if (bind(sockfd, (struct sockaddr *)&addr, sizeof(struct sockaddr_nl)) <
	    0) {
		perror("Failed to bind socket");
		close(sockfd);
		return -1;
	}

	// Get socket addr
	if (getsockname(sockfd, (struct sockaddr *)&addr, &addrlen) < 0) {
		perror("Failed to get socket name");
		close(sockfd);
		return -1;
	}

	// Print socket address
	printf("Socket address: %d\n", addr.nl_pid);

	close(sockfd);

	return 0;
}
