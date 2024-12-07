// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
#include <netinet/ip.h>
#include <netinet/udp.h>

#define BUFFER_SIZE 1024

int main()
{
	int sockfd;
	char buffer[BUFFER_SIZE];

	// Create RAW socket
	if ((sockfd = socket(AF_INET, SOCK_RAW, IPPROTO_UDP)) < 0) {
		perror("Server: socket creation failed");
		return 1;
	}

	printf("Server: Waiting for UDP packets...\n");

	// Receive RAW UDP packet
	struct sockaddr_in src_addr;
	socklen_t src_addr_len = sizeof(src_addr);
	ssize_t len = recvfrom(sockfd, buffer, BUFFER_SIZE, 0,
			       (struct sockaddr *)&src_addr, &src_addr_len);
	if (len < 0) {
		perror("Server: recvfrom failed");
		close(sockfd);
		return 1;
	}

	struct iphdr *ip_header = (struct iphdr *)buffer;
	struct udphdr *udp_header =
		(struct udphdr *)(buffer + (ip_header->ihl * 4));
	char *data = buffer + (ip_header->ihl * 4) + sizeof(struct udphdr);

	printf("Server: Received RAW UDP packet\n");
	printf("Server: Source IP: %s\n",
	       inet_ntoa(*(struct in_addr *)&ip_header->saddr));
	printf("Server: Source Port: %d\n", ntohs(udp_header->source));
	printf("Server: Data: '%s'\n", data);

	close(sockfd);
	return 0;
}
