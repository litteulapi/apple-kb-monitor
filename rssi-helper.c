/*
 * rssi-helper - RSSI/TX power of a connected BR/EDR device via BlueZ MGMT.
 *
 * MGMT GET_CONN_INFO (opcode 0x0031) is refused to unprivileged sockets
 * (status 0x14 PERMISSION_DENIED), so this is the ONLY binary of the package
 * that carries CAP_NET_ADMIN (file capability set by the .install script).
 * The GUI (apihub-app) stays unprivileged and runs this helper as a child.
 *
 * Access control (#209): installed root:akm 0750 by the .install script, so only
 * the members of the "akm" group (the user who reads the signal) can run it.
 * Compromise: the group must be joined once (usermod -aG akm) and a new login is
 * needed; in return a local account that is not in it can no longer probe the
 * presence and proximity (RSSI, TX power) of the phones / headsets connected to
 * this machine. The code itself is strict (MAC format, paired answer, AT_SECURE)
 * and does not read the MGMT events that follow the command.
 *
 * Usage:   rssi-helper AA:BB:CC:DD:EE:FF [hci_index]
 * Success: stdout = {"rssi":-5,"tx_power":4,"max_tx_power":4}, exit 0
 * Failure: stdout empty, stderr = "rssi-helper: <reason>", exit code:
 *            1 usage / bad argument      2 socket, bind or send error
 *            3 MGMT status (stderr gives the hex status, 20 = 0x14 no CAP_NET_ADMIN,
 *              0x0d not connected)       4 timeout, no reply
 *            5 RSSI / TX power not available (127)
 *
 * Part of apple-kb-monitor - GPL-2.0-or-later
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <ctype.h>
#include <time.h>
#include <stdint.h>
#include <errno.h>
#include <string.h>

#define BTPROTO_HCI         1
#define HCI_CHANNEL_CONTROL 3
#define MGMT_OP_GET_CONN_INFO 0x0031
#define MGMT_EV_CMD_COMPLETE  0x0001
#define MGMT_EV_CMD_STATUS    0x0002
#define AF_BLUETOOTH_NUM    31
#define MGMT_VALUE_INVALID  127   /* RSSI / TX power not available */

#pragma pack(push, 1)
struct mgmt_hdr { uint16_t opcode, index, len; };
struct mgmt_cp  { uint8_t addr[6], addr_type; };
#pragma pack(pop)

/* Strict "AA:BB:CC:DD:EE:FF" parser: exactly 17 chars, two hex digits per octet. */
static int parse_mac(const char *s, unsigned int b[6])
{
    if (strlen(s) != 17) return -1;
    for (int i = 0; i < 6; i++) {
        const char *p = s + i * 3;
        if (!isxdigit((unsigned char)p[0]) || !isxdigit((unsigned char)p[1])) return -1;
        if (i < 5 && p[2] != ':') return -1;
        char tmp[3] = { p[0], p[1], 0 };
        b[i] = (unsigned int)strtoul(tmp, NULL, 16);
    }
    return 0;
}

static uint16_t rd16(const uint8_t *p) { return (uint16_t)(p[0] | (p[1] << 8)); }

int main(int argc, char *argv[])
{
    if (argc < 2 || argc > 3) { fprintf(stderr, "Usage: %s MAC [hci_index]\n", argv[0]); return 1; }
    unsigned int b[6];
    if (parse_mac(argv[1], b) != 0) {
        fprintf(stderr, "rssi-helper: bad MAC\n"); return 1;
    }
    unsigned long idx = 0;
    if (argc == 3) {
        char *end = NULL;
        idx = strtoul(argv[2], &end, 10);
        if (*argv[2] == '\0' || *end != '\0' || idx > 0xFFFE) {
            fprintf(stderr, "rssi-helper: bad controller index\n"); return 1;
        }
    }

    int fd = socket(AF_BLUETOOTH_NUM, SOCK_RAW | SOCK_CLOEXEC, BTPROTO_HCI);
    if (fd < 0) { fprintf(stderr, "rssi-helper: socket: %s\n", strerror(errno)); return 2; }

    struct { sa_family_t f; uint16_t dev, ch; } sa = {AF_BLUETOOTH_NUM, 0xFFFF, HCI_CHANNEL_CONTROL};
    if (bind(fd, (void*)&sa, sizeof(sa)) < 0) {
        fprintf(stderr, "rssi-helper: bind: %s\n", strerror(errno)); close(fd); return 2;
    }

    struct timeval tv = {.tv_sec = 0, .tv_usec = 200000};
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));

    struct mgmt_cp cp = {.addr_type = 0};
    for (int i = 0; i < 6; i++) cp.addr[i] = (uint8_t)b[5-i];  /* LE byte order */
    struct mgmt_hdr h = {MGMT_OP_GET_CONN_INFO, (uint16_t)idx, sizeof(cp)};

    uint8_t buf[256];
    memcpy(buf, &h, sizeof(h)); memcpy(buf + sizeof(h), &cp, sizeof(cp));
    if (send(fd, buf, sizeof(h) + sizeof(cp), 0) < 0) {
        fprintf(stderr, "rssi-helper: send: %s\n", strerror(errno)); close(fd); return 2;
    }

    /* The control channel also carries unrelated events: read until the reply
     * to our own opcode arrives (bounded by a 400 ms deadline). */
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (;;) {
        ssize_t n = recv(fd, buf, sizeof(buf), 0);
        if (n < 0 && errno == EINTR) continue;
        if (n < 0) break;                            /* receive timeout / error */
        if (n >= 9) {
            uint16_t ev = rd16(buf), op = rd16(buf + 6);
            if ((ev == MGMT_EV_CMD_COMPLETE || ev == MGMT_EV_CMD_STATUS) &&
                op == MGMT_OP_GET_CONN_INFO) {
                close(fd);
                if (buf[8] != 0 || ev == MGMT_EV_CMD_STATUS || n < 19) {
                    fprintf(stderr, "rssi-helper: MGMT status 0x%02x%s\n", buf[8],
                            buf[8] == 0x14 ? " (permission denied: CAP_NET_ADMIN missing)" :
                            buf[8] == 0x0d ? " (not connected)" : "");
                    return 3;
                }
                if ((int8_t)buf[16] == MGMT_VALUE_INVALID) {
                    fprintf(stderr, "rssi-helper: RSSI not available\n");
                    return 5;
                }
                /* TX power / max TX power may legitimately be "not available"
                 * (127): report null instead of discarding a valid RSSI. */
                char tx[8], mtx[8];
                if ((int8_t)buf[17] == MGMT_VALUE_INVALID) snprintf(tx, sizeof tx, "null");
                else snprintf(tx, sizeof tx, "%d", (int8_t)buf[17]);
                if ((int8_t)buf[18] == MGMT_VALUE_INVALID) snprintf(mtx, sizeof mtx, "null");
                else snprintf(mtx, sizeof mtx, "%d", (int8_t)buf[18]);
                printf("{\"rssi\":%d,\"tx_power\":%s,\"max_tx_power\":%s}\n",
                       (int8_t)buf[16], tx, mtx);
                return 0;
            }
        }
        clock_gettime(CLOCK_MONOTONIC, &t1);
        long ms = (t1.tv_sec - t0.tv_sec) * 1000 + (t1.tv_nsec - t0.tv_nsec) / 1000000;
        if (ms > 400) break;
    }
    close(fd);
    fprintf(stderr, "rssi-helper: timeout, no reply from bluetoothd/kernel\n");
    return 4;
}
