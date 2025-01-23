// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
#include <netinet/ip.h>
#include <netinet/udp.h>

#define DEST_IP "127.0.0.1"
#define DEST_PORT 12345
#define BUFFER_SIZE 1024

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

// Pseudo-header structure for UDP checksum calculation
struct pseudo_header {
	unsigned int src_addr;
	unsigned int dest_addr;
	unsigned char placeholder;
	unsigned char protocol;
	unsigned short udp_length;
};

int main()
{
	int sockfd;
	struct sockaddr_in dest_addr;
	char packet[BUFFER_SIZE];

	// Create RAW socket
	if ((sockfd = socket(AF_INET, SOCK_RAW, IPPROTO_UDP)) < 0) {
		perror("Raw UDP client: Socket creation failed");
		exit(EXIT_FAILURE);
	}

	// Enable IP_HDRINCL
	int on = 1;
	if (setsockopt(sockfd, IPPROTO_IP, IP_HDRINCL, &on, sizeof(on)) < 0) {
		perror("Raw UDP client: Setsockopt failed");
		close(sockfd);
		exit(EXIT_FAILURE);
	}

	// Set destination address
	memset(&dest_addr, 0, sizeof(dest_addr));
	dest_addr.sin_family = AF_INET;
	dest_addr.sin_port = htons(DEST_PORT);
	inet_pton(AF_INET, DEST_IP, &dest_addr.sin_addr);

	// Construct IP header
	struct iphdr *ip_header = (struct iphdr *)packet;
	struct udphdr *udp_header =
		(struct udphdr *)(packet + sizeof(struct iphdr));
	char *data = packet + sizeof(struct iphdr) + sizeof(struct udphdr);
	memset(packet, 0, sizeof(packet));

	// Fill the IP header
	ip_header->ihl = 5;
	ip_header->version = 4;
	ip_header->tos = 0;
	ip_header->tot_len =
		htons(sizeof(struct iphdr) + sizeof(struct udphdr) +
		      strlen("Hello from RAW UDP client!"));
	ip_header->id = htons(54321);
	ip_header->frag_off = 0;
	ip_header->ttl = 64;
	ip_header->protocol = IPPROTO_UDP;
	ip_header->saddr = INADDR_ANY; // Source IP will be filled by the kernel
	ip_header->daddr = dest_addr.sin_addr.s_addr;

	// Calculate IP checksum
	ip_header->check = checksum(ip_header, sizeof(struct iphdr));

	// Fill the UDP header
	udp_header->source = htons(54321);
	udp_header->dest = htons(DEST_PORT);
	udp_header->len = htons(sizeof(struct udphdr) +
				strlen("Hello from RAW UDP client!"));
	udp_header->check = 0; // Initially zero

	// Copy data
	strcpy(data, "Hello from RAW UDP client!");

	struct pseudo_header psh;
	psh.src_addr = ip_header->saddr;
	psh.dest_addr = ip_header->daddr;
	psh.placeholder = 0;
	psh.protocol = IPPROTO_UDP;
	psh.udp_length = htons(sizeof(struct udphdr) + strlen(data));

	char pseudo_packet[BUFFER_SIZE];
	memcpy(pseudo_packet, &psh, sizeof(psh));
	memcpy(pseudo_packet + sizeof(psh), udp_header,
	       sizeof(struct udphdr) + strlen(data));

	// Calculate UDP checksum
	udp_header->check =
		checksum(pseudo_packet,
			 sizeof(psh) + sizeof(struct udphdr) + strlen(data));

	// Send the packet
	if (sendto(sockfd, packet, ntohs(ip_header->tot_len), 0,
		   (struct sockaddr *)&dest_addr, sizeof(dest_addr)) < 0) {
		perror("Raw UDP client: sendto failed");
		close(sockfd);
		exit(EXIT_FAILURE);
	}
	printf("Raw UDP client: Sent RAW UDP packet\n");

	close(sockfd);
	return 0;
}
