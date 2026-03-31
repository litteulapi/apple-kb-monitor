/*
 * rssi-helper — Tiny CAP_NET_ADMIN helper for BlueZ MGMT RSSI reads.
 *
 * Sends MGMT GET_CONN_INFO (opcode 0x0031) on the HCI control channel
 * and prints RSSI, TX power as JSON to stdout.
 *
 * Install: gcc -O2 -o rssi-helper rssi-helper.c
 *          setcap cap_net_admin+ep rssi-helper
 *
 * Usage: rssi-helper AA:BB:CC:DD:EE:FF
 * Output: {"rssi":-5,"tx_power":4,"max_tx_power":4}
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

#define BTPROTO_HCI       1
#define HCI_CHANNEL_CONTROL 3

#define MGMT_OP_GET_CONN_INFO 0x0031
#define MGMT_EV_CMD_COMPLETE  0x0001

struct mgmt_hdr {
    uint16_t opcode;
    uint16_t index;
    uint16_t len;
} __attribute__((packed));

struct mgmt_cp_get_conn_info {
    uint8_t addr[6];
    uint8_t addr_type;
} __attribute__((packed));

static int parse_mac(const char *str, uint8_t out[6])
{
    unsigned int b[6];
    if (sscanf(str, "%02x:%02x:%02x:%02x:%02x:%02x",
               &b[0], &b[1], &b[2], &b[3], &b[4], &b[5]) != 6)
        return -1;
    /* Store in little-endian (reversed) for MGMT */
    for (int i = 0; i < 6; i++)
        out[i] = (uint8_t)b[5 - i];
    return 0;
}

int main(int argc, char *argv[])
{
    if (argc != 2) {
        fprintf(stderr, "Usage: %s AA:BB:CC:DD:EE:FF\n", argv[0]);
        return 1;
    }

    uint8_t addr_le[6];
    if (parse_mac(argv[1], addr_le) < 0) {
        fprintf(stderr, "Invalid MAC: %s\n", argv[1]);
        return 1;
    }

    int fd = socket(31 /* PF_BLUETOOTH */, SOCK_RAW, BTPROTO_HCI);
    if (fd < 0) {
        fprintf(stderr, "{\"error\":\"socket: %s\"}\n", strerror(errno));
        return 1;
    }

    struct sockaddr_hci {
        sa_family_t hci_family;
        unsigned short hci_dev;
        unsigned short hci_channel;
    } addr = {
        .hci_family = 31,
        .hci_dev = 0xFFFF,
        .hci_channel = HCI_CHANNEL_CONTROL,
    };

    if (bind(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        fprintf(stderr, "{\"error\":\"bind: %s\"}\n", strerror(errno));
        close(fd);
        return 1;
    }

    /* Set 2 second timeout */
    struct timeval tv = { .tv_sec = 2 };
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));

    /* Build GET_CONN_INFO command */
    struct mgmt_cp_get_conn_info cp;
    memcpy(cp.addr, addr_le, 6);
    cp.addr_type = 0x00; /* BR/EDR */

    struct mgmt_hdr hdr = {
        .opcode = MGMT_OP_GET_CONN_INFO,
        .index = 0, /* hci0 */
        .len = sizeof(cp),
    };

    uint8_t buf[256];
    memcpy(buf, &hdr, sizeof(hdr));
    memcpy(buf + sizeof(hdr), &cp, sizeof(cp));

    if (send(fd, buf, sizeof(hdr) + sizeof(cp), 0) < 0) {
        fprintf(stderr, "{\"error\":\"send: %s\"}\n", strerror(errno));
        close(fd);
        return 1;
    }

    ssize_t n = recv(fd, buf, sizeof(buf), 0);
    close(fd);

    if (n < 19) {
        fprintf(stderr, "{\"error\":\"short response\"}\n");
        return 1;
    }

    uint16_t ev = *(uint16_t *)buf;
    if (ev != MGMT_EV_CMD_COMPLETE) {
        fprintf(stderr, "{\"error\":\"unexpected event 0x%04x\"}\n", ev);
        return 1;
    }

    uint8_t status = buf[8];
    if (status != 0) {
        fprintf(stderr, "{\"error\":\"status %d\"}\n", status);
        return 1;
    }

    int8_t rssi     = (int8_t)buf[16];
    int8_t tx_power = (int8_t)buf[17];
    int8_t max_tx   = (int8_t)buf[18];

    printf("{\"rssi\":%d,\"tx_power\":%d,\"max_tx_power\":%d}\n",
           rssi, tx_power, max_tx);
    return 0;
}
