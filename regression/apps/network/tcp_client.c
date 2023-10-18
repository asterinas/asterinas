// From: https://www.geeksforgeeks.org/socket-programming-cc/.
// Some minor modifications are made to the original code base.
// Lisenced under CCBY-SA.

// Client side C/C++ program to demonstrate socket programming
#include <arpa/inet.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>
#include <stdlib.h>
#define PORT 8080
  
int main(int argc, char const* argv[])
{
    if(argc < 2) {
        perror("Server address is not provided.");
        exit(EXIT_FAILURE);
    }

    int status, valread, client_fd;
    struct sockaddr_in serv_addr;
    char* hello = "Hello from client";
    char buffer[1024] = { 0 };
    if ((client_fd = socket(AF_INET, SOCK_STREAM, 0)) < 0) {
        printf("\n Socket creation error \n");
        return -1;
    }
  
    serv_addr.sin_family = AF_INET;
    serv_addr.sin_port = htons(PORT);
  
    // Convert IPv4 and IPv6 addresses from text to binary
    // form
    if (inet_pton(AF_INET, argv[1], &serv_addr.sin_addr)
        <= 0) {
        printf(
            "\nInvalid address/ Address not supported \n");
        return -1;
    }
  
    if ((status
         = connect(client_fd, (struct sockaddr*)&serv_addr,
                   sizeof(serv_addr)))
        < 0) {
        printf("\nConnection Failed \n");
        return -1;
    }

    struct sockaddr_in peer_addr;
    socklen_t peer_addr_len = sizeof(peer_addr);
    if (getpeername(client_fd, (struct sockaddr*)&peer_addr, &peer_addr_len) == -1) {         
        perror("Getpeername failed");            
        exit(EXIT_FAILURE);
    }

    // Get peername
    char peer_ip_addr[INET_ADDRSTRLEN];
    if (inet_ntop(AF_INET, &(peer_addr.sin_addr), peer_ip_addr, INET_ADDRSTRLEN) == NULL) {
        perror("inet_ntop failed");
        exit(EXIT_FAILURE);
    }

    printf("Client: server IP address: %s\n", peer_ip_addr);
    printf("Client: server port: %d\n", ntohs(peer_addr.sin_port));

    send(client_fd, hello, strlen(hello), 0);
    printf("Hello message sent\n");
    valread = read(client_fd, buffer, 1024);
    printf("%s\n", buffer);
  
    // closing the connected socket
    close(client_fd);
    return 0;
}