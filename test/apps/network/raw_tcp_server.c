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
		perror("Raw TCP server: Socket creation failed");
		exit(EXIT_FAILURE);
	}

	// Receive and print incoming packets
	ssize_t packet_size = recvfrom(sock, buffer, sizeof(buffer), 0,
				       (struct sockaddr *)&client_addr,
				       &client_len);
	if (packet_size < 0) {
		perror("Raw TCP server: Error receiving packet");
		close(sock);
		exit(EXIT_FAILURE);
	}

	// Extract IP header
	struct iphdr *iph = (struct iphdr *)buffer;
	struct tcphdr *tcph = (struct tcphdr *)(buffer + iph->ihl * 4);

	printf("Raw TCP server: Received packet from %s:%d to %s:%d, Sequence Number: %u!\n",
	       inet_ntoa(*(struct in_addr *)&iph->saddr), ntohs(tcph->source),
	       inet_ntoa(*(struct in_addr *)&iph->daddr), ntohs(tcph->dest),
	       ntohl(tcph->seq));

	close(sock);
	return 0;
}
