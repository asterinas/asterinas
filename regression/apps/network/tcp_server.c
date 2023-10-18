// From: https://www.geeksforgeeks.org/socket-programming-cc/.
// Some minor modifications are made to the original code base.
// Lisenced under CCBY-SA.

// Server side C/C++ program to demonstrate socket programming
#include <arpa/inet.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>
#define PORT 8080
int main(int argc, char const* argv[])
{
    if(argc < 2) {
        perror("Server address is not provided.");
        exit(EXIT_FAILURE);
    }

    int server_fd, new_socket, valread;
    struct sockaddr_in address;
    int opt = 1;
    int addrlen = sizeof(address);
    char buffer[1024] = { 0 };
    char* hello = "Hello from server";
  
    // Creating socket file descriptor
    if ((server_fd = socket(AF_INET, SOCK_STREAM, 0)) < 0) {
        perror("socket failed");
        exit(EXIT_FAILURE);
    }
  
    // Forcefully attaching socket to the port 8080
    if (setsockopt(server_fd, SOL_SOCKET,
                   SO_REUSEADDR | SO_REUSEPORT, &opt,
                   sizeof(opt))) {
        perror("setsockopt");
        exit(EXIT_FAILURE);
    }
    address.sin_family = AF_INET;
    address.sin_port = htons(PORT);
    // Convert IPv4 and IPv6 addresses from text to binary form
    if (inet_pton(AF_INET, argv[1], &address.sin_addr)
        <= 0) {
        printf(
            "\nInvalid address/ Address not supported \n");
        return -1;
    }
  
    // Forcefully attaching socket to the port 8080
    if (bind(server_fd, (struct sockaddr*)&address,
             sizeof(address))
        < 0) {
        perror("bind failed");
        exit(EXIT_FAILURE);
    }
    if (listen(server_fd, 3) < 0) {
        perror("listen failed");
        exit(EXIT_FAILURE);
    }
    if ((new_socket
         = accept(server_fd, (struct sockaddr*)&address,
                  (socklen_t*)&addrlen))
        < 0) {
        perror("accept");
        exit(EXIT_FAILURE);
    }
    valread = read(new_socket, buffer, 1024);
    printf("%s\n", buffer);
    send(new_socket, hello, strlen(hello), 0);
    printf("Hello message sent\n");

    struct sockaddr_in peer_addr;
    socklen_t peer_addr_len = sizeof(peer_addr);
    if (getpeername(new_socket, (struct sockaddr*)&peer_addr, &peer_addr_len) == -1) {         
        perror("Getpeername failed");            
        exit(EXIT_FAILURE);
    }

    // Get peername
    char peer_ip_addr[INET_ADDRSTRLEN];
    if (inet_ntop(AF_INET, &(peer_addr.sin_addr), peer_ip_addr, INET_ADDRSTRLEN) == NULL) {
        perror("inet_ntop failed");
        exit(EXIT_FAILURE);
    }

    printf("Server: client IP address: %s\n", peer_ip_addr);
    printf("Server: client port: %d\n", ntohs(peer_addr.sin_port));
  
    // closing the connected socket
    close(new_socket);
    // closing the listening socket
    shutdown(server_fd, SHUT_RDWR);
    return 0;
}