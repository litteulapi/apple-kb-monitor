/*
 * rssi-helper — RSSI/TX Power via BlueZ MGMT API (modern interface).
 *
 * Uses MGMT GET_CONN_INFO (opcode 0x0031) — the official BlueZ API
 * for radio diagnostics. No deprecated HCI raw access.
 *
 * Requires: CAP_NET_ADMIN (setcap cap_net_admin+ep rssi-helper)
 * Usage:    rssi-helper AA:BB:CC:DD:EE:FF
 * Output:   {"rssi":-5,"tx_power":4,"max_tx_power":4}
 *
 * Part of apple-kb-monitor — GPL-2.0-or-later
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

#define BTPROTO_HCI         1
#define HCI_CHANNEL_CONTROL 3
#define MGMT_OP_GET_CONN_INFO 0x0031
#define MGMT_EV_CMD_COMPLETE  0x0001
#define MGMT_EV_CMD_STATUS    0x0002
#define AF_BLUETOOTH_NUM    31
#define MGMT_VALUE_INVALID  127   /* RSSI / TX power not available */
#define NULL_JSON "{\"rssi\":null,\"tx_power\":null,\"max_tx_power\":null}\n"

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
    if (argc != 2) { fprintf(stderr, "Usage: %s MAC\n", argv[0]); return 1; }
    unsigned int b[6];
    if (parse_mac(argv[1], b) != 0) {
        fprintf(stderr, "Bad MAC\n"); return 1;
    }

    int fd = socket(AF_BLUETOOTH_NUM, SOCK_RAW | SOCK_CLOEXEC, BTPROTO_HCI);
    if (fd < 0) { perror("socket"); return 1; }

    struct { sa_family_t f; uint16_t dev, ch; } sa = {AF_BLUETOOTH_NUM, 0xFFFF, HCI_CHANNEL_CONTROL};
    if (bind(fd, (void*)&sa, sizeof(sa)) < 0) { perror("bind"); close(fd); return 1; }

    struct timeval tv = {.tv_sec = 0, .tv_usec = 200000};
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));

    struct mgmt_cp cp = {.addr_type = 0};
    for (int i = 0; i < 6; i++) cp.addr[i] = (uint8_t)b[5-i];  /* LE byte order */
    struct mgmt_hdr h = {MGMT_OP_GET_CONN_INFO, 0, sizeof(cp)};

    uint8_t buf[256];
    memcpy(buf, &h, sizeof(h)); memcpy(buf + sizeof(h), &cp, sizeof(cp));
    if (send(fd, buf, sizeof(h) + sizeof(cp), 0) < 0) { perror("send"); close(fd); return 1; }

    /* The control channel also carries unrelated events: read until the reply
     * to our own opcode arrives (bounded by a 400 ms deadline). */
    struct timespec t0, t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (;;) {
        ssize_t n = recv(fd, buf, sizeof(buf), 0);
        if (n < 0) {
            if (errno == EINTR) continue;
            break;                                   /* timeout / error */
        }
        clock_gettime(CLOCK_MONOTONIC, &t1);
        long ms = (t1.tv_sec - t0.tv_sec) * 1000 + (t1.tv_nsec - t0.tv_nsec) / 1000000;
        if (n >= 9) {
            uint16_t ev = rd16(buf), op = rd16(buf + 6);
            if ((ev == MGMT_EV_CMD_COMPLETE || ev == MGMT_EV_CMD_STATUS) &&
                op == MGMT_OP_GET_CONN_INFO) {
                if (ev == MGMT_EV_CMD_COMPLETE && buf[8] == 0 && n >= 19 &&
                    (int8_t)buf[16] != MGMT_VALUE_INVALID &&
                    (int8_t)buf[17] != MGMT_VALUE_INVALID) {
                    close(fd);
                    printf("{\"rssi\":%d,\"tx_power\":%d,\"max_tx_power\":%d}\n",
                           (int8_t)buf[16], (int8_t)buf[17], (int8_t)buf[18]);
                    return 0;
                }
                break;                               /* our reply, but an error */
            }
        }
        if (ms > 400) break;
    }
    close(fd);
    printf(NULL_JSON);
    return 0;
}
