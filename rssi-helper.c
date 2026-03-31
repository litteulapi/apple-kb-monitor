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
#include <stdint.h>
#include <errno.h>

#define BTPROTO_HCI         1
#define HCI_CHANNEL_CONTROL 3
#define MGMT_OP_GET_CONN_INFO 0x0031
#define MGMT_EV_CMD_COMPLETE  0x0001

#pragma pack(push, 1)
struct mgmt_hdr { uint16_t opcode, index, len; };
struct mgmt_cp  { uint8_t addr[6], addr_type; };
#pragma pack(pop)

int main(int argc, char *argv[])
{
    if (argc != 2) { fprintf(stderr, "Usage: %s MAC\n", argv[0]); return 1; }
    unsigned int b[6];
    if (sscanf(argv[1], "%02x:%02x:%02x:%02x:%02x:%02x",
               &b[0],&b[1],&b[2],&b[3],&b[4],&b[5]) != 6) {
        fprintf(stderr, "Bad MAC\n"); return 1;
    }

    int fd = socket(31, SOCK_RAW, BTPROTO_HCI);
    if (fd < 0) { perror("socket"); return 1; }

    struct { sa_family_t f; uint16_t dev, ch; } sa = {31, 0xFFFF, HCI_CHANNEL_CONTROL};
    if (bind(fd, (void*)&sa, sizeof(sa)) < 0) { perror("bind"); close(fd); return 1; }

    struct timeval tv = {.tv_sec = 2};
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));

    struct mgmt_cp cp = {.addr_type = 0};
    for (int i = 0; i < 6; i++) cp.addr[i] = b[5-i];  /* LE byte order */
    struct mgmt_hdr h = {MGMT_OP_GET_CONN_INFO, 0, sizeof(cp)};

    uint8_t buf[256];
    memcpy(buf, &h, 6); memcpy(buf+6, &cp, 7);
    if (send(fd, buf, 13, 0) < 0) { perror("send"); close(fd); return 1; }

    ssize_t n = recv(fd, buf, sizeof(buf), 0);
    close(fd);
    if (n < 19 || *(uint16_t*)buf != MGMT_EV_CMD_COMPLETE || buf[8] != 0) {
        printf("{\"rssi\":null,\"tx_power\":null,\"max_tx_power\":null}\n");
        return 0;
    }
    printf("{\"rssi\":%d,\"tx_power\":%d,\"max_tx_power\":%d}\n",
           (int8_t)buf[16], (int8_t)buf[17], (int8_t)buf[18]);
    return 0;
}
