// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <arpa/inet.h>
#include <netinet/ip.h>
#include <netinet/tcp.h>
#include <sys/socket.h>
#include <unistd.h>

// Calculate checksum
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
	int sock;
	char packet[4096];
	struct sockaddr_in dest;

	// Create a raw socket
	sock = socket(AF_INET, SOCK_RAW, IPPROTO_TCP);
	if (sock < 0) {
		perror("Raw TCP client: Socket creation failed");
		exit(EXIT_FAILURE);
	}

	// Enable IP_HDRINCL to manually construct the IP header
	int optval = 1;
	if (setsockopt(sock, IPPROTO_IP, IP_HDRINCL, &optval, sizeof(optval)) <
	    0) {
		perror("Raw TCP client: Error setting IP_HDRINCL");
		close(sock);
		exit(EXIT_FAILURE);
	}

	// Set the destination address
	dest.sin_family = AF_INET;
	dest.sin_port = htons(80);
	dest.sin_addr.s_addr = inet_addr("127.0.0.1");
	memset(packet, 0, sizeof(packet));

	// Construct the IP header
	struct iphdr *iph = (struct iphdr *)packet;
	struct tcphdr *tcph = (struct tcphdr *)(packet + sizeof(struct iphdr));

	iph->version = 4; // IPv4
	iph->ihl = 5; // Header length
	iph->tos = 0; // Type of service
	iph->tot_len = htons(sizeof(struct iphdr) +
			     sizeof(struct tcphdr)); // Total length
	iph->id = htonl(54321); // ID
	iph->frag_off = 0; // Fragment offset
	iph->ttl = 64; // Time to live
	iph->protocol = IPPROTO_TCP; // Protocol
	iph->saddr = inet_addr("127.0.0.1"); // Source IP
	iph->daddr = dest.sin_addr.s_addr; // Destination IP
	iph->check = checksum(iph, sizeof(struct iphdr)); // IP header checksum

	// Construct the TCP header
	tcph->source = htons(12345); // Source port
	tcph->dest = htons(80); // Destination port
	tcph->seq = htonl(0); // Sequence number
	tcph->ack_seq = 0; // Acknowledgment number
	tcph->doff = 5; // Data offset
	tcph->fin = 0; // FIN flag
	tcph->syn = 1; // SYN flag
	tcph->rst = 0; // RST flag
	tcph->psh = 0; // PSH flag
	tcph->ack = 0; // ACK flag
	tcph->urg = 0; // URG flag
	tcph->window = htons(5840); // Window size
	tcph->check = 0; // Checksum (computed later)
	tcph->urg_ptr = 0; // Urgent pointer

	// Compute TCP checksum
	struct {
		unsigned int src_addr;
		unsigned int dest_addr;
		unsigned char placeholder;
		unsigned char protocol;
		unsigned short tcp_length;
		struct tcphdr tcp;
	} pseudo_header;

	pseudo_header.src_addr = iph->saddr;
	pseudo_header.dest_addr = iph->daddr;
	pseudo_header.placeholder = 0;
	pseudo_header.protocol = IPPROTO_TCP;
	pseudo_header.tcp_length = htons(sizeof(struct tcphdr));
	memcpy(&pseudo_header.tcp, tcph, sizeof(struct tcphdr));

	tcph->check = checksum(&pseudo_header, sizeof(pseudo_header));

	// Send the packet
	if (sendto(sock, packet, sizeof(struct iphdr) + sizeof(struct tcphdr),
		   0, (struct sockaddr *)&dest, sizeof(dest)) < 0) {
		perror("Raw TCP client: Packet send failed");
		close(sock);
		exit(EXIT_FAILURE);
	} else {
		printf("Raw TCP client: Packet sent successfully!\n");
	}

	close(sock);
	return 0;
}
