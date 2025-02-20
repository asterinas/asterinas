#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <linux/netlink.h>
#include <linux/rtnetlink.h>
#include <arpa/inet.h>
#include <net/if.h>

#define BUFFER_SIZE 4096

struct nl_req {
	struct nlmsghdr hdr;
	struct ifaddrmsg ifa;
};

void parse_rtattr(struct rtattr *tb[], int max, struct rtattr *rta, int len)
{
	memset(tb, 0, sizeof(struct rtattr *) * (max + 1));
	while (RTA_OK(rta, len)) {
		if (rta->rta_type <= max) {
			tb[rta->rta_type] = rta;
		}
		rta = RTA_NEXT(rta, len);
	}
}

int main()
{
	struct sockaddr_nl sa;
	int fd, ret;
	char buffer[BUFFER_SIZE];
	struct nlmsghdr *nlh;
	struct nl_req req;

	// 创建 Netlink 套接字
	fd = socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE);
	if (fd < 0) {
		perror("socket");
		return -1;
	}

	// 绑定套接字
	memset(&sa, 0, sizeof(sa));
	sa.nl_family = AF_NETLINK;
	sa.nl_groups = 0;

	if (bind(fd, (struct sockaddr *)&sa, sizeof(sa)) < 0) {
		perror("bind");
		close(fd);
		return -1;
	}

	// 构造请求消息
	memset(&req, 0, sizeof(req));
	req.hdr.nlmsg_len = NLMSG_LENGTH(sizeof(struct ifaddrmsg));
	req.hdr.nlmsg_type = RTM_GETADDR;
	req.hdr.nlmsg_flags = NLM_F_REQUEST | NLM_F_DUMP;
	req.hdr.nlmsg_seq = 1;
	req.ifa.ifa_family = AF_UNSPEC; // 获取所有地址族

	// 发送请求
	ret = send(fd, &req, req.hdr.nlmsg_len, 0);
	if (ret < 0) {
		perror("send");
		close(fd);
		return -1;
	}

	// 接收响应
	ssize_t len;
	while ((len = recv(fd, buffer, sizeof(buffer), 0)) > 0) {
		for (nlh = (struct nlmsghdr *)buffer; NLMSG_OK(nlh, len);
		     nlh = NLMSG_NEXT(nlh, len)) {
			if (nlh->nlmsg_type == NLMSG_ERROR) {
				fprintf(stderr, "Netlink error\n");
				close(fd);
				return -1;
			}

			if (nlh->nlmsg_type == NLMSG_DONE) {
				close(fd);
				return 0;
			}

			if (nlh->nlmsg_type == RTM_NEWADDR) {
				struct ifaddrmsg *ifa =
					(struct ifaddrmsg *)NLMSG_DATA(nlh);
				struct rtattr *tb[IFA_MAX + 1];
				int ifa_len = nlh->nlmsg_len -
					      NLMSG_LENGTH(sizeof(*ifa));
				char ifname[IF_NAMESIZE] = { 0 };
				char addr_str[INET6_ADDRSTRLEN] = { 0 };

				parse_rtattr(tb, IFA_MAX, IFA_RTA(ifa),
					     ifa_len);

				// 获取接口名称
				if (tb[IFA_LABEL]) {
					char *name =
						(char *)RTA_DATA(tb[IFA_LABEL]);
					strncpy(ifname, name, IF_NAMESIZE - 1);
				}

				// 获取 IP 地址
				if (tb[IFA_ADDRESS]) {
					void *addr = RTA_DATA(tb[IFA_ADDRESS]);
					const char *inet_family =
						(ifa->ifa_family == AF_INET) ?
							"IPv4" :
							"IPv6";

					inet_ntop(ifa->ifa_family, addr,
						  addr_str, sizeof(addr_str));
					printf("Interface: %-8s Family: %-4s Address: %s\n",
					       ifname, inet_family, addr_str);
				}
			}
		}
	}

	close(fd);
	return 0;
}
