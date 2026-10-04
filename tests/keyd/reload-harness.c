/*
 * reload-harness.c - drive keyd's daemon event handler without any device.
 *
 * Built by tests/keyd/run-harness.sh against the keyd 2.6.0 sources, with
 * -fsanitize=address,undefined. It #includes src/daemon.c so the static
 * functions (reload, event_handler) are reachable, and replaces everything
 * that touches the system: no /dev/uinput (vkbd/stdout.c backend), no
 * /dev/input (fake device_table, device_grab() is a no-op), no IPC socket,
 * no evloop. Nothing is grabbed, nothing is written to a keyboard.
 *
 * usage: reload-harness CONFIG_DIR SCENARIO
 *   parse     load the configs, match the fake Apple keyboard, type a key
 *   mouse     ... then reload, then click an unmanaged mouse  (test PC crash)
 *   timeout   ... then reload, then deliver a pending timeout
 *   rekey     ... then reload, then type on the keyboard again
 *
 * Exit 0 = survived. A use-after-free makes ASAN abort (exit != 0).
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

char harness_config_dir[1024];
#define CONFIG_DIR harness_config_dir

#include "daemon.c"

/* ---- stubs for device.c / evloop.c (the real ones open /dev/input) ---- */
struct device device_table[MAX_DEVICES];
size_t device_table_sz;

int device_grab(struct device *dev) { dev->grabbed = 1; return 0; }
int device_ungrab(struct device *dev) { dev->grabbed = 0; return 0; }
void device_set_led(const struct device *dev, int led, int state) { (void)dev; (void)led; (void)state; }
struct device_event *device_read_event(struct device *dev) { (void)dev; return NULL; }
int device_scan(struct device devices[MAX_DEVICES]) { (void)devices; return 0; }
int devmon_create(void) { return -1; }
int devmon_read_device(int fd, struct device *dev) { (void)fd; (void)dev; return -1; }
void evloop_add_fd(int fd) { (void)fd; }
int evloop(int (*handler)(struct event *ev)) { (void)handler; return 0; }

static int now = 1000;

static void add_dev(struct device *d, const char *id, const char *name, uint8_t caps)
{
	memset(d, 0, sizeof *d);
	d->fd = -1;
	d->capabilities = caps;
	snprintf(d->id, sizeof d->id, "%s", id);
	snprintf(d->name, sizeof d->name, "%s", name);
	snprintf(d->path, sizeof d->path, "/dev/input/fake-%s", id);

	struct event ev = { .type = EV_DEV_ADD, .dev = d, .timestamp = now };
	event_handler(&ev);
}

static void dev_key(struct device *d, uint8_t code, uint8_t pressed)
{
	struct device_event de = { .type = DEV_KEY, .code = code, .pressed = pressed };
	struct event ev = { .type = EV_DEV_EVENT, .dev = d, .devev = &de, .timestamp = now };
	now += 20;
	event_handler(&ev);
}

static void do_reload(void)
{
	/* Exactly what IPC_RELOAD (`keyd reload`) does in handle_client(). */
	fprintf(stderr, "harness: reload (IPC_RELOAD)\n");
	reload();
}

int main(int argc, char *argv[])
{
	if (argc != 3) {
		fprintf(stderr, "usage: %s CONFIG_DIR parse|mouse|timeout|rekey\n", argv[0]);
		return 64;
	}
	snprintf(harness_config_dir, sizeof harness_config_dir, "%s", argv[1]);
	const char *sc = argv[2];

	vkbd = vkbd_init("keyd harness");
	reload();	/* startup path of run_daemon() */

	device_table_sz = 2;
	struct device *kb = &device_table[0], *mouse = &device_table[1];
	/* The two devices of the test PC journal at 12:35:10. */
	add_dev(kb, "05ac:0256:09409bbc", "Clavier de alice #1", CAP_KEY | CAP_KEYBOARD);
	add_dev(mouse, "1d57:fa60:3c7f9e03", "2.4G Wireless Device", CAP_MOUSE | CAP_KEY);

	if (!kb->data) {
		fprintf(stderr, "harness: the config does not match 05ac:0256 (no keyboard managed)\n");
	}

	/* The user types on the Apple keyboard: active_kbd = that keyboard. */
	dev_key(kb, KEYD_A, 1);
	dev_key(kb, KEYD_A, 0);

	if (!strcmp(sc, "parse")) {
		/* nothing else */
	} else if (!strcmp(sc, "mouse")) {
		do_reload();
		dev_key(mouse, KEYD_LEFT_MOUSE, 1);	/* -> process_keypress(active_kbd, KEYD_EXTERNAL_MOUSE_BUTTON) */
		dev_key(mouse, KEYD_LEFT_MOUSE, 0);
	} else if (!strcmp(sc, "timeout")) {
		do_reload();
		struct event ev = { .type = EV_TIMEOUT, .timestamp = now + 500 };
		event_handler(&ev);		/* -> kbd_process_events(active_kbd, ...) */
	} else if (!strcmp(sc, "rekey")) {
		do_reload();
		dev_key(kb, KEYD_A, 1);
		dev_key(kb, KEYD_A, 0);
		dev_key(mouse, KEYD_LEFT_MOUSE, 1);
		dev_key(mouse, KEYD_LEFT_MOUSE, 0);
	} else {
		fprintf(stderr, "unknown scenario %s\n", sc);
		return 64;
	}

	free_configs();
	printf("harness: scenario %s survived\n", sc);
	return 0;
}
