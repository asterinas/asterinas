// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <arpa/inet.h>
#include <netinet/ip.h>
#include <netinet/tcp.h>
#include <sys/socket.h>
#include <unistd.h>

int main()
{
	int sock;
	char buffer[4096];
	struct sockaddr_in client_addr;
	socklen_t client_len = sizeof(client_addr);

	// Create a raw socket to listen for incoming TCP packets
	sock = socket(AF_INET, SOCK_RAW, IPPROTO_TCP);
	if (sock < 0) {
		perror("Socket creation failed");
		return EXIT_FAILURE;
	}

	printf("Server is listening for raw TCP packets...\n");

	// Infinite loop to receive and print incoming packets
	ssize_t packet_size = recvfrom(sock, buffer, sizeof(buffer), 0,
				       (struct sockaddr *)&client_addr,
				       &client_len);
	if (packet_size < 0) {
		perror("Error receiving packet");
		exit(-1);
	}

	// Extract IP header
	struct iphdr *iph = (struct iphdr *)buffer;
	struct tcphdr *tcph = (struct tcphdr *)(buffer + iph->ihl * 4);

	printf("Received packet:\n");
	printf("Source IP: %s\n", inet_ntoa(*(struct in_addr *)&iph->saddr));
	printf("Destination IP: %s\n",
	       inet_ntoa(*(struct in_addr *)&iph->daddr));
	printf("Source Port: %d\n", ntohs(tcph->source));
	printf("Destination Port: %d\n", ntohs(tcph->dest));
	printf("Sequence Number: %u\n", ntohl(tcph->seq));
	printf("Acknowledgment Number: %u\n", ntohl(tcph->ack_seq));
	printf("Flags: SYN=%d, ACK=%d, FIN=%d, RST=%d, PSH=%d\n", tcph->syn,
	       tcph->ack, tcph->fin, tcph->rst, tcph->psh);

	close(sock);
	return 0;
}
