// SPDX-License-Identifier: MPL-2.0

#include <stdio.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <string.h>
#include <unistd.h>
#include <arpa/inet.h>

#define PORT 9090
#define BUFFER_SIZE 1024

int main()
{
	int sockfd, client_socket;
	struct sockaddr_in server_address, client_address;
	char buffer[BUFFER_SIZE] = { 0 };
	struct msghdr msg = { 0 };
	struct iovec iov[1];

	// Create socket
	if ((sockfd = socket(AF_INET, SOCK_STREAM, 0)) == 0) {
		perror("socket failed");
		exit(EXIT_FAILURE);
	}

	// Set server address and port
	server_address.sin_family = AF_INET;
	server_address.sin_port = htons(PORT);
	if (inet_pton(AF_INET, "127.0.0.1", &(server_address.sin_addr)) <= 0) {
		perror("Invalid address: Address not supported");
		exit(EXIT_FAILURE);
	}

	// Bind socket to specified address and port
	if (bind(sockfd, (struct sockaddr *)&server_address,
		 sizeof(server_address)) < 0) {
		perror("bind failed");
		exit(EXIT_FAILURE);
	}

	// Listen for connections
	if (listen(sockfd, 3) < 0) {
		perror("listen failed");
		exit(EXIT_FAILURE);
	}

	printf("Server listening on port %d\n", PORT);

	// Accept client connection
	int client_address_length = sizeof(client_address);
	if ((client_socket = accept(sockfd, (struct sockaddr *)&client_address,
				    (socklen_t *)&client_address_length)) < 0) {
		perror("accept failed");
		exit(EXIT_FAILURE);
	}

	// Receive message from the client
	iov[0].iov_base = buffer;
	iov[0].iov_len = sizeof(buffer);
	msg.msg_iov = iov;
	msg.msg_iovlen = 1;

	ssize_t received_bytes = recvmsg(client_socket, &msg, 0);
	printf("Received message from client: %.*s\n", (int)received_bytes,
	       buffer);

	// Send response to the client
	char *response = "This is the server's response.";
	iov[0].iov_base = response;
	iov[0].iov_len = strlen(response);
	msg.msg_iov = iov;
	msg.msg_iovlen = 1;

	sendmsg(client_socket, &msg, 0);
	printf("Sent response to client: %s\n", response);

	// Close sockets
	close(client_socket);
	close(sockfd);

	return 0;
}
