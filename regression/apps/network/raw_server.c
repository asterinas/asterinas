#include <arpa/inet.h>
#include <netinet/ip.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

#define BUFFER_SIZE 1024

int main() {
  int sock_fd;
  ssize_t bytes_received;
  char buffer[BUFFER_SIZE];
  struct sockaddr_in server_addr;
  struct sockaddr_in client_addr;
  socklen_t addr_len;

  // Create a raw socket
  if ((sock_fd = socket(AF_INET, SOCK_RAW, IPPROTO_UDP)) == -1) {
    perror("socket creation failed");
    exit(EXIT_FAILURE);
  }

  // Initialize server address structure
  memset(&server_addr, 0, sizeof(server_addr));
  server_addr.sin_family = AF_INET;
  server_addr.sin_addr.s_addr = inet_addr("127.0.0.1");

  // Bind socket to local address
  if (bind(sock_fd, (struct sockaddr *)&server_addr, sizeof(server_addr)) ==
      -1) {
    perror("bind failed");
    close(sock_fd);
    exit(EXIT_FAILURE);
  }

  printf("Server is running...\n");

  // Receive messages from client
  while (1) {
    addr_len = sizeof(client_addr);
    bytes_received = recvfrom(sock_fd, buffer, BUFFER_SIZE, 0,
                              (struct sockaddr *)&client_addr, &addr_len);
    if (bytes_received > 0) {
      printf("Received a packet from %s\n", inet_ntoa(client_addr.sin_addr));
      // Process packet here
    }
  }

  // Close the socket
  close(sock_fd);
  return 0;
}
