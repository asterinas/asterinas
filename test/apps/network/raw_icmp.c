// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <arpa/inet.h>
#include <sys/socket.h>
#include <netinet/ip.h>
#include <netinet/ip_icmp.h>
#include <errno.h>

// Maximum size of the ICMP packet
#define PACKET_SIZE 64

// Function to calculate checksum
unsigned short checksum(void *b, int len)
{
	unsigned short *buf = b;
	unsigned int sum = 0;
	unsigned short result;

	for (sum = 0; len > 1; len -= 2) {
		sum += *buf++;
	}
	if (len == 1) {
		sum += *(unsigned char *)buf;
	}
	sum = (sum >> 16) + (sum & 0xFFFF);
	sum += (sum >> 16);
	result = ~sum;
	return result;
}

int main()
{
	int sockfd;
	struct sockaddr_in dest_addr;
	char packet[PACKET_SIZE];
	struct iphdr *ip_hdr = (struct iphdr *)packet;
	struct icmphdr *icmp_hdr =
		(struct icmphdr *)(packet + sizeof(struct iphdr));
	char recv_buf[PACKET_SIZE];
	int recv_sum = 0;

	// Create a raw socket
	if ((sockfd = socket(AF_INET, SOCK_RAW, IPPROTO_ICMP)) < 0) {
		perror("Socket creation failed");
		exit(EXIT_FAILURE);
	}

	// Enable the IP_HDRINCL option to construct the IP header manually
	int opt = 1;
	if (setsockopt(sockfd, IPPROTO_IP, IP_HDRINCL, &opt, sizeof(opt)) < 0) {
		perror("Setsockopt failed");
		close(sockfd);
		exit(EXIT_FAILURE);
	}

	// Set the destination address to 127.0.0.1
	memset(&dest_addr, 0, sizeof(dest_addr));
	dest_addr.sin_family = AF_INET;
	dest_addr.sin_addr.s_addr = inet_addr("127.0.0.1");

	// Clear the packet buffer
	memset(packet, 0, PACKET_SIZE);

	// Construct the IP header
	ip_hdr->ihl = 5; // Header length
	ip_hdr->version = 4; // IPv4
	ip_hdr->tos = 0; // Type of Service
	ip_hdr->tot_len = htons(sizeof(struct iphdr) +
				sizeof(struct icmphdr)); // Total packet length
	ip_hdr->id = htons(54321); // Packet ID
	ip_hdr->frag_off = 0; // No fragmentation
	ip_hdr->ttl = 64; // Time-to-Live
	ip_hdr->protocol = IPPROTO_ICMP; // Protocol (ICMP)
	ip_hdr->check = 0; // Initialize checksum to 0
	ip_hdr->saddr = inet_addr("127.0.0.1"); // Source address
	ip_hdr->daddr = dest_addr.sin_addr.s_addr; // Destination address
	ip_hdr->check = checksum((unsigned short *)ip_hdr,
				 sizeof(struct iphdr)); // Compute checksum

	// Construct the ICMP header
	icmp_hdr->type = ICMP_ECHO; // ICMP Echo Request
	icmp_hdr->code = 0; // Code 0
	icmp_hdr->checksum = 0; // Initialize checksum to 0
	icmp_hdr->un.echo.id = htons(1234); // Identifier
	icmp_hdr->un.echo.sequence = htons(1); // Sequence number

	// Compute the ICMP checksum
	icmp_hdr->checksum =
		checksum((unsigned short *)icmp_hdr, sizeof(struct icmphdr));

	// Send the packet
	if (sendto(sockfd, packet,
		   sizeof(struct iphdr) + sizeof(struct icmphdr), 0,
		   (struct sockaddr *)&dest_addr, sizeof(dest_addr)) < 0) {
		perror("Packet send failed");
		close(sockfd);
		exit(EXIT_FAILURE);
	}
	printf("ICMP Echo Request sent to 127.0.0.1\n");

	// Receive the response
	struct sockaddr_in recv_addr;
	socklen_t addr_len = sizeof(recv_addr);
	ssize_t recv_len;
	while (1) {
		if ((recv_len = recvfrom(sockfd, recv_buf, PACKET_SIZE, 0,
					 (struct sockaddr *)&recv_addr,
					 &addr_len)) < 0) {
			perror("Packet receive failed");
			close(sockfd);
			exit(EXIT_FAILURE);
		}

		// Parse the received packet
		struct iphdr *recv_ip_hdr = (struct iphdr *)recv_buf;
		struct icmphdr *recv_icmp_hdr =
			(struct icmphdr *)(recv_buf + (recv_ip_hdr->ihl * 4));

		// Verify if it's an ICMP Echo Reply
		if (recv_icmp_hdr->type == ICMP_ECHOREPLY &&
		    ntohs(recv_icmp_hdr->un.echo.id) == 1234) {
			printf("Received ICMP Echo Reply from %s, Sequence: %d!\n",
			       inet_ntoa(recv_addr.sin_addr),
			       ntohs(recv_icmp_hdr->un.echo.sequence));
			recv_sum++;
			if (recv_sum == 2)
				break;
		} else if (recv_icmp_hdr->type == ICMP_ECHO) {
			printf("Received ICMP Echo Request packet which is sent to loopback!\n");
			recv_sum++;
			if (recv_sum == 2)
				break;
		} else {
			printf("Received the packet, but it is neither an Echo Request nor Echo Reply packet!\n");
			close(sockfd);
			exit(EXIT_FAILURE);
		}
	}

	// Close the socket
	close(sockfd);

	return 0;
}
