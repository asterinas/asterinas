// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
#include <netinet/ip.h>
#include <netinet/ip_icmp.h>

#define DEST_IP "10.0.2.15"
#define BUFFER_SIZE 1024

// Test raw icmp with IP_HDRINCL
// Function to calculate checksum
unsigned short checksum(void *b, int len)
{
	unsigned short *buf = b;
	unsigned int sum = 0;
	unsigned short result;

	for (sum = 0; len > 1; len -= 2)
		sum += *buf++;
	if (len == 1)
		sum += *(unsigned char *)buf;
	sum = (sum >> 16) + (sum & 0xFFFF);
	sum += (sum >> 16);
	result = ~sum;
	return result;
}

int main()
{
	int sockfd;
	struct sockaddr_in dest_addr;
	char packet[BUFFER_SIZE];

	// Create RAW socket
	if ((sockfd = socket(AF_INET, SOCK_RAW, IPPROTO_ICMP)) < 0) {
		perror("socket creation failed");
		return 1;
	}

	// Enable IP_HDRINCL
	int on = 1;
	if (setsockopt(sockfd, IPPROTO_IP, IP_HDRINCL, &on, sizeof(on)) < 0) {
		perror("setsockopt failed");
		close(sockfd);
		return 1;
	}

	// Set destination address
	memset(&dest_addr, 0, sizeof(dest_addr));
	dest_addr.sin_family = AF_INET;
	inet_pton(AF_INET, DEST_IP, &dest_addr.sin_addr);

	// Construct IP header
	struct iphdr *ip_header = (struct iphdr *)packet;
	struct icmphdr *icmp_header =
		(struct icmphdr *)(packet + sizeof(struct iphdr));
	memset(packet, 0, sizeof(packet));

	ip_header->ihl = 5;
	ip_header->version = 4;
	ip_header->tos = 0;
	ip_header->tot_len =
		htons(sizeof(struct iphdr) + sizeof(struct icmphdr));
	ip_header->id = htons(54321);
	ip_header->frag_off = 0;
	ip_header->ttl = 64;
	ip_header->protocol = IPPROTO_ICMP;
	ip_header->saddr = INADDR_ANY;
	ip_header->daddr = dest_addr.sin_addr.s_addr;

	// Calculate IP checksum
	ip_header->check = checksum(ip_header, sizeof(struct iphdr));

	// Construct ICMP header
	icmp_header->type = ICMP_ECHO;
	icmp_header->code = 0;
	icmp_header->un.echo.id = htons(12345);
	icmp_header->un.echo.sequence = htons(1);
	icmp_header->checksum = 0;
	icmp_header->checksum = checksum(icmp_header, sizeof(struct icmphdr));

	// Send the packet
	if (sendto(sockfd, packet,
		   sizeof(struct iphdr) + sizeof(struct icmphdr), 0,
		   (struct sockaddr *)&dest_addr, sizeof(dest_addr)) < 0) {
		perror("sendto failed");
		close(sockfd);
		return 1;
	}
	printf("Sent ICMP Echo Request\n");

	// Receive server response
	char buffer[BUFFER_SIZE];
	ssize_t len = recv(sockfd, buffer, BUFFER_SIZE, 0);
	if (len < 0) {
		perror("recv failed");
		close(sockfd);
		return 1;
	}

	struct iphdr *recv_ip_header = (struct iphdr *)buffer;
	struct icmphdr *recv_icmp_header =
		(struct icmphdr *)(buffer + (recv_ip_header->ihl * 4));

	if (recv_icmp_header->type == ICMP_ECHOREPLY) {
		printf("Received ICMP Echo Reply\n");
	} else {
		printf("Received non-Echo Reply packet\n");
	}

	close(sockfd);
	return 0;
}
