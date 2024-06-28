// SPDX-License-Identifier: MPL-2.0

#include <signal.h>
#include <stdlib.h>
#include <time.h>
#include "mongoose.h"

static int s_debug_level = MG_LL_INFO;
static const char *s_listening_address = "http://127.0.0.1:8080/";

static int s_signo;
static void signal_handler(int signo)
{
	s_signo = signo;
}

// Event handler for the listening connection.
static void cb(struct mg_connection *c, int ev, void *ev_data)
{
	if (ev == MG_EV_HTTP_MSG) {
		struct mg_http_message *hm = ev_data;
		if (mg_match(hm->uri, mg_str("/"), NULL)) {
			// Generate a random number
			srand(time(NULL));
			int random_number = rand();
			char response[100];
			sprintf(response, "Random number: %d\n", random_number);
			MG_INFO(("Send a random number : %d", random_number));
			mg_http_reply(c, 200, "", "%s", response);
		} else {
			// Serve 404 for other routes
			mg_http_reply(c, 404, "", "Not found");
		}
		// Remove this line if you need a long running server
		// signal_handler(SIGTERM);
	}
}

int main(int argc, char *argv[])
{
	struct mg_mgr mgr;
	struct mg_connection *c;

	// Parse command-line flags
	for (int i = 1; i < argc; i++) {
		if (strcmp(argv[i], "-l") == 0) {
			s_listening_address = argv[++i];
		} else if (strcmp(argv[i], "-v") == 0) {
			s_debug_level = atoi(argv[++i]);
		}
	}

	// Initialise stuff
	signal(SIGINT, signal_handler);
	signal(SIGTERM, signal_handler);
	mg_log_set(s_debug_level);
	mg_mgr_init(&mgr);
	if ((c = mg_http_listen(&mgr, s_listening_address, cb, &mgr)) == NULL) {
		MG_ERROR(("Cannot listen on %s. Use http://ADDR:PORT or :PORT",
			  s_listening_address));
		exit(EXIT_FAILURE);
	}

	// Start infinite event loop
	MG_INFO(("Mongoose version : v%s", MG_VERSION));
	MG_INFO(("Listening on     : %s", s_listening_address));
	while (s_signo == 0)
		mg_mgr_poll(&mgr, 1000);
	mg_mgr_free(&mgr);
	MG_INFO(("Exiting on signal %d", s_signo));
	return 0;
}