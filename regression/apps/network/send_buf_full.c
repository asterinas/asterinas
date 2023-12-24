#include <arpa/inet.h>
#include <netinet/in.h>
#include <stdio.h>
#include <sys/socket.h>
#include <sys/wait.h>
#include <sched.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>

static int new_bound_socket(struct sockaddr_in *addr)
{
    int sockfd;

    sockfd = socket(PF_INET, SOCK_STREAM, 0);
    if (sockfd < 0) {
        perror("new_bound_socket: socket");
        return -1;
    }

    if (bind(sockfd, (struct sockaddr *)addr, sizeof(*addr)) < 0) {
        perror("new_bound_socket: bind");
        close(sockfd);
        return -1;
    }

    return sockfd;
}

static int new_connected_socket(struct sockaddr_in *addr)
{
    int sockfd;

    sockfd = socket(PF_INET, SOCK_STREAM, 0);
    if (sockfd < 0) {
        perror("new_connected_socket: socket");
        return -1;
    }

    if (connect(sockfd, (struct sockaddr *)addr, sizeof(*addr)) < 0) {
        perror("new_connected_socket: connect");
        close(sockfd);
        return -1;
    }

    return sockfd;
}

static int accept_without_addr(int sockfd)
{
    struct sockaddr addr;
    socklen_t addrlen = sizeof(addr);
    int acceptfd;

    acceptfd = accept(sockfd, &addr, &addrlen);
    if (acceptfd < 0) {
        perror("accept_without_addr: accept");
        return -1;
    }

    return acceptfd;
}

static int mark_filde_nonblock(int flide)
{
    int flags;

    flags = fcntl(flide, F_GETFL, 0);
    if (flags < 0) {
        perror("mark_filde_nonblock: fcntl(F_GETFL)");
        return -1;
    }

    if (fcntl(flide, F_SETFL, flags | O_NONBLOCK) < 0) {
        perror("mark_filde_nonblock: fcntl(F_SETFL)");
        return -1;
    }

    return 0;
}

static int mark_filde_mayblock(int flide)
{
    int flags;

    flags = fcntl(flide, F_GETFL, 0);
    if (flags < 0) {
        perror("mark_filde_mayblock: fcntl(F_GETFL)");
        return -1;
    }

    if (fcntl(flide, F_SETFL, flags & ~O_NONBLOCK) < 0) {
        perror("mark_filde_mayblock: fcntl(F_SETFL)");
        return -1;
    }

    return 0;
}

static char buffer[4096] = "Hello, world";

static ssize_t receive_all(int sockfd)
{
    size_t recv_len = 0;
    ssize_t ret;

    if (mark_filde_nonblock(sockfd) < 0) {
        perror("receive_all: mark_filde_nonblock");
        return -1;
    }

    for (;;) {
        ret = recv(sockfd, buffer, sizeof(buffer), 0);
        if (ret < 0 && errno == EAGAIN)
            break;

        if (ret < 0) {
            perror("receive_all: recv");
            return -1;
        }

        recv_len += ret;
    }

    return recv_len;
}

int test_full_send_buffer(struct sockaddr_in *addr)
{
    int listenfd, sendfd, recvfd;
    int ret = -1, wstatus;
    size_t sent_len = 0;
    ssize_t sent;
    int pid;

    listenfd = new_bound_socket(addr);
    if (listenfd < 0) {
        fprintf(stderr,
            "Test failed: Error occurs in new_bound_socket\n");
        return -1;
    }

    if (listen(listenfd, 2) < 0) {
        perror("listen");
        fprintf(stderr, "Test failed: Error occurs in listen\n");
        goto out_listen;
    }

    sendfd = new_connected_socket(addr);
    if (sendfd < 0) {
        fprintf(stderr,
            "Test failed: Error occurs in new_connected_socket\n");
        goto out_listen;
    }

    recvfd = accept_without_addr(listenfd);
    if (recvfd < 0) {
        fprintf(stderr,
            "Test failed: Error occurs in accept_without_addr\n");
        goto out_send;
    }

    if (mark_filde_nonblock(sendfd) < 0) {
        fprintf(stderr,
            "Test failed: Error occurs in mark_filde_nonblock\n");
        goto out;
    }

    for (;;) {
        sent = send(sendfd, buffer, sizeof(buffer), 0);
        if (sent < 0 && errno == EAGAIN)
            break;

        if (sent < 0) {
            perror("send");
            fprintf(stderr, "Test failed: Error occurs in send\n");
            goto out;
        }

        sent_len += sent;
    }

    if (mark_filde_mayblock(sendfd) < 0) {
        fprintf(stderr,
            "Test failed: Error occurs in mark_filde_mayblock\n");
        goto out;
    }

    pid = fork();
    if (pid < 0) {
        perror("fork");
        fprintf(stderr, "Test failed: Error occurs in fork\n");
        goto out;
    }

    if (pid == 0) {
        int i;
        ssize_t recv_len;

        // Ensure that the parent executes send() first, then the child
        // executes recv().
        sleep(1);

        fprintf(stderr, "Start receiving...\n");
        recv_len = receive_all(recvfd);
        if (recv_len < 0) {
            fprintf(stderr,
                "Test failed: Error occurs in receive_all\n");
            goto out;
        }

        fprintf(stderr, "Received bytes: %lu\n", recv_len);
        if (recv_len != sent_len + 1) {
            fprintf(stderr,
                "Test failed: Mismatched sent bytes and received bytes\n");
            goto out;
        }

        ret = 0;
        goto out;
    }

    sent = send(sendfd, buffer, 1, 0);
    if (sent < 0) {
        perror("send");
        fprintf(stderr, "Test failed: Error occurs in send\n");
        goto wait;
    }

    sent_len += 1;
    fprintf(stderr, "Sent bytes: %lu\n", sent_len);

    ret = 0;

wait:
    if (wait(&wstatus) < 0) {
        perror("wait");
        fprintf(stderr, "Test failed: Error occurs in wait\n");
        ret = -1;
    } else if (WEXITSTATUS(wstatus) != 0) {
        fprintf(stderr, "Test failed: Error occurs in child process\n");
        ret = -1;
    }

    if (ret == 0)
        fprintf(stderr,
            "Test passed: Equal sent bytes and received bytes\n");

out:
    close(recvfd);

out_send:
    close(sendfd);

out_listen:
    close(listenfd);

    return ret;
}

int main(void)
{
    struct sockaddr_in addr;

    addr.sin_family = AF_INET;
    addr.sin_port = htons(8080);
    if (inet_aton("127.0.0.1", &addr.sin_addr) < 0) {
        fprintf(stderr, "inet_aton cannot parse 127.0.0.1\n");
        return -1;
    }

    return test_full_send_buffer(&addr);
}
