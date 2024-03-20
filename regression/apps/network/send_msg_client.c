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
	int sockfd;
	struct sockaddr_in server_address;
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

	// Connect to the server
	if (connect(sockfd, (struct sockaddr *)&server_address,
		    sizeof(server_address)) < 0) {
		perror("connection failed");
		exit(EXIT_FAILURE);
	}

	// Send message
	char *message = "This is a message from the client.";
	iov[0].iov_base = message;
	iov[0].iov_len = strlen(message);
	msg.msg_iov = iov;
	msg.msg_iovlen = 1;

	sendmsg(sockfd, &msg, 0);
	printf("Sent message to server: %s\n", message);

	// Receive response from the server
	iov[0].iov_base = buffer;
	iov[0].iov_len = BUFFER_SIZE;
	msg.msg_iov = iov;
	msg.msg_iovlen = 1;
	recvmsg(sockfd, &msg, 0);
	printf("Received message from server: %s\n", buffer);

	// Close socket
	close(sockfd);

	return 0;
}
