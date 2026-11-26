#import <AppKit/AppKit.h>
#import <CoreFoundation/CoreFoundation.h>
#import <Foundation/Foundation.h>
#import <IOBluetooth/IOBluetooth.h>

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/types.h>

typedef void (*MurmurDeviceCallback)(
    void *ctx,
    const char *name_utf8,
    const char *addr_utf8,
    int is_connected,
    uint32_t class_of_device
);

// ---------------------------------------------------------------------------
// Dedicated IOBluetooth RunLoop thread
// ---------------------------------------------------------------------------

static NSThread *g_bt_thread = nil;
static CFRunLoopRef g_bt_runloop = NULL;

@interface MurmurBtThreadTarget : NSObject
- (void)threadMain:(dispatch_semaphore_t)readySem;
@end

@implementation MurmurBtThreadTarget
- (void)threadMain:(dispatch_semaphore_t)readySem {
    @autoreleasepool {
        NSRunLoop *rl = [NSRunLoop currentRunLoop];
        [rl addPort:[NSPort port] forMode:NSDefaultRunLoopMode];
        g_bt_runloop = [rl getCFRunLoop];
        CFRetain(g_bt_runloop);
        dispatch_semaphore_signal(readySem);
        while (![[NSThread currentThread] isCancelled]) {
            @autoreleasepool {
                [rl runMode:NSDefaultRunLoopMode beforeDate:[NSDate distantFuture]];
            }
        }
    }
}
@end

static void murmur_ensure_bt_thread(void) {
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        dispatch_semaphore_t ready = dispatch_semaphore_create(0);
        MurmurBtThreadTarget *target = [[MurmurBtThreadTarget alloc] init];
        g_bt_thread = [[NSThread alloc] initWithTarget:target
                                              selector:@selector(threadMain:)
                                                object:ready];
        [g_bt_thread setName:@"murmur-iobluetooth-runloop"];
        [g_bt_thread start];
        dispatch_semaphore_wait(ready, DISPATCH_TIME_FOREVER);
    });
}

static void murmur_run_on_bt_thread(void (^block)(void)) {
    murmur_ensure_bt_thread();
    if ([NSThread currentThread] == g_bt_thread) {
        block();
        return;
    }
    dispatch_semaphore_t done = dispatch_semaphore_create(0);
    CFRunLoopPerformBlock(g_bt_runloop, kCFRunLoopCommonModes, ^{
        @autoreleasepool {
            block();
        }
        dispatch_semaphore_signal(done);
    });
    CFRunLoopWakeUp(g_bt_runloop);
    dispatch_semaphore_wait(done, DISPATCH_TIME_FOREVER);
}

static void murmur_set_err(char *err_buf, size_t err_len, NSString *msg) {
    if (!err_buf || err_len == 0) {
        return;
    }
    const char *cstr = msg ? [msg UTF8String] : "Unknown macOS Bluetooth error";
    if (!cstr) {
        cstr = "Unknown macOS Bluetooth error";
    }
    snprintf(err_buf, err_len, "%s", cstr);
}

static NSString *murmur_normalize_colon_addr(NSString *raw) {
    if (!raw) {
        return @"";
    }
    return [[raw stringByReplacingOccurrencesOfString:@"-" withString:@":"] uppercaseString];
}

static IOBluetoothDevice *murmur_find_device(const char *addr_utf8) {
    if (!addr_utf8) {
        return nil;
    }
    NSString *input = [NSString stringWithUTF8String:addr_utf8];
    if (!input || input.length == 0) {
        return nil;
    }
    NSString *hyphen = [[input stringByReplacingOccurrencesOfString:@":" withString:@"-"] lowercaseString];
    IOBluetoothDevice *dev = [IOBluetoothDevice deviceWithAddressString:hyphen];
    if (dev) {
        return dev;
    }
    dev = [IOBluetoothDevice deviceWithAddressString:input];
    if (dev) {
        return dev;
    }
    NSString *target = murmur_normalize_colon_addr(input);
    for (IOBluetoothDevice *paired in [IOBluetoothDevice pairedDevices]) {
        if ([murmur_normalize_colon_addr(paired.addressString) isEqualToString:target]) {
            return paired;
        }
    }
    return nil;
}

static int murmur_hex_nibble(char c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return 10 + (c - 'a');
    if (c >= 'A' && c <= 'F') return 10 + (c - 'A');
    return -1;
}

static IOBluetoothSDPUUID *murmur_sdp_uuid_from_cstr(const char *uuid_utf8) {
    if (!uuid_utf8) {
        return nil;
    }
    uint8_t bytes[16];
    size_t count = 0;
    const char *p = uuid_utf8;
    while (*p) {
        if (*p == '-' || *p == ' ' || *p == '{' || *p == '}') {
            p++;
            continue;
        }
        int hi = murmur_hex_nibble(p[0]);
        int lo = p[1] ? murmur_hex_nibble(p[1]) : -1;
        if (hi < 0 || lo < 0 || count >= sizeof(bytes)) {
            return nil;
        }
        bytes[count++] = (uint8_t)((hi << 4) | lo);
        p += 2;
    }
    if (count == 2 || count == 4 || count == 16) {
        return [IOBluetoothSDPUUID uuidWithBytes:bytes length:(unsigned)count];
    }
    return nil;
}

// ---------------------------------------------------------------------------
// Theme, Device Enumeration & Apple Battery KVC
// ---------------------------------------------------------------------------

int murmur_macos_is_dark_mode(void) {
    @autoreleasepool {
        NSString *style = [[NSUserDefaults standardUserDefaults] stringForKey:@"AppleInterfaceStyle"];
        if (style && [style caseInsensitiveCompare:@"Dark"] == NSOrderedSame) {
            return 1;
        }
        return 0;
    }
}

void murmur_macos_enumerate_paired_devices(MurmurDeviceCallback cb, void *ctx) {
    if (!cb) {
        return;
    }
    murmur_run_on_bt_thread(^{
        NSArray *paired = [IOBluetoothDevice pairedDevices];
        for (IOBluetoothDevice *dev in paired) {
            NSString *addr = murmur_normalize_colon_addr(dev.addressString);
            if (addr.length == 0) {
                continue;
            }
            NSString *name = dev.nameOrAddress ?: addr;
            int connected = [dev isConnected] ? 1 : 0;
            uint32_t cod = (uint32_t)[dev classOfDevice];
            cb(ctx, [name UTF8String], [addr UTF8String], connected, cod);
        }
    });
}

static int murmur_kvc_battery_percent(IOBluetoothDevice *dev, NSString *key) {
    @try {
        id val = [dev valueForKey:key];
        if ([val isKindOfClass:[NSNumber class]]) {
            int pct = [(NSNumber *)val intValue];
            if (pct >= 0 && pct <= 100) {
                return pct;
            }
        }
    } @catch (NSException *ignored) {
        (void)ignored;
    }
    return -1;
}

int murmur_macos_read_apple_battery(
    const char *addr_utf8,
    int *out_left,
    int *out_right,
    int *out_case,
    int *out_single
) {
    if (!out_left || !out_right || !out_case || !out_single) {
        return 0;
    }
    *out_left = -1;
    *out_right = -1;
    *out_case = -1;
    *out_single = -1;

    __block int found = 0;
    murmur_run_on_bt_thread(^{
        IOBluetoothDevice *dev = murmur_find_device(addr_utf8);
        if (!dev) {
            return;
        }
        int left = murmur_kvc_battery_percent(dev, @"batteryPercentLeft");
        int right = murmur_kvc_battery_percent(dev, @"batteryPercentRight");
        int case_pct = murmur_kvc_battery_percent(dev, @"batteryPercentCase");
        int single = murmur_kvc_battery_percent(dev, @"batteryPercentSingle");
        if (single < 0) {
            single = murmur_kvc_battery_percent(dev, @"batteryPercentCombined");
        }
        *out_left = left;
        *out_right = right;
        *out_case = case_pct;
        *out_single = single;
        if (left >= 0 || right >= 0 || case_pct >= 0 || single >= 0) {
            found = 1;
        }
    });
    return found;
}

// ---------------------------------------------------------------------------
// RFCOMM Transport Bridge
// ---------------------------------------------------------------------------

@interface MurmurRfcommHandle : NSObject <IOBluetoothRFCOMMChannelDelegate>
@property (nonatomic, strong) IOBluetoothRFCOMMChannel *channel;
@property (nonatomic, strong) NSCondition *cond;
@property (nonatomic, strong) NSMutableData *rxBuffer;
@property (nonatomic, assign) BOOL closed;
@end

@implementation MurmurRfcommHandle

- (instancetype)init {
    self = [super init];
    if (self) {
        _cond = [[NSCondition alloc] init];
        _rxBuffer = [[NSMutableData alloc] init];
        _closed = NO;
    }
    return self;
}

- (void)rfcommChannelData:(IOBluetoothRFCOMMChannel *)rfcommChannel
                     data:(void *)dataPointer
                   length:(size_t)dataLength {
    (void)rfcommChannel;
    if (!dataPointer || dataLength == 0) {
        return;
    }
    [self.cond lock];
    if (!self.closed) {
        [self.rxBuffer appendBytes:dataPointer length:dataLength];
    }
    [self.cond broadcast];
    [self.cond unlock];
}

- (void)rfcommChannelClosed:(IOBluetoothRFCOMMChannel *)rfcommChannel {
    (void)rfcommChannel;
    [self.cond lock];
    self.closed = YES;
    [self.cond broadcast];
    [self.cond unlock];
}

@end

void *murmur_rfcomm_connect(
    const char *addr_utf8,
    const char *uuid_utf8,
    char *err_buf,
    size_t err_len
) {
    __block void *result = NULL;
    murmur_run_on_bt_thread(^{
        IOBluetoothDevice *dev = murmur_find_device(addr_utf8);
        if (!dev) {
            murmur_set_err(err_buf, err_len, @"Bluetooth device not found in paired macOS devices");
            return;
        }

        IOBluetoothSDPUUID *sdpUuid = murmur_sdp_uuid_from_cstr(uuid_utf8);
        if (!sdpUuid) {
            murmur_set_err(err_buf, err_len, @"Invalid RFCOMM service UUID");
            return;
        }

        IOBluetoothSDPServiceRecord *record = [dev getServiceRecordForUUID:sdpUuid];
        if (!record) {
            [dev performSDPQuery:nil];
            record = [dev getServiceRecordForUUID:sdpUuid];
        }
        if (!record) {
            murmur_set_err(
                err_buf,
                err_len,
                @"SDP service record not found for requested UUID (ensure headphones are connected)"
            );
            return;
        }

        BluetoothRFCOMMChannelID channelId = 0;
        IOReturn sdpStatus = [record getRFCOMMChannelID:&channelId];
        if (sdpStatus != kIOReturnSuccess || channelId == 0) {
            murmur_set_err(
                err_buf,
                err_len,
                [NSString stringWithFormat:@"Service record has no RFCOMM channel ID (0x%x)", sdpStatus]
            );
            return;
        }

        MurmurRfcommHandle *handle = [[MurmurRfcommHandle alloc] init];
        IOBluetoothRFCOMMChannel *chan = nil;
        IOReturn openStatus = [dev openRFCOMMChannelSync:&chan
                                            withChannelID:channelId
                                                 delegate:handle];
        if (openStatus != kIOReturnSuccess || !chan) {
            murmur_set_err(
                err_buf,
                err_len,
                [NSString stringWithFormat:@"openRFCOMMChannelSync(ch=%u) failed (0x%x)", channelId, openStatus]
            );
            return;
        }

        handle.channel = chan;
        result = (void *)CFBridgingRetain(handle);
    });
    return result;
}

ssize_t murmur_rfcomm_read(void *ptr, uint8_t *buf, size_t cap, uint32_t timeout_ms) {
    if (!ptr || !buf || cap == 0) {
        return -1;
    }
    MurmurRfcommHandle *handle = (__bridge MurmurRfcommHandle *)ptr;
    [handle.cond lock];
    while (handle.rxBuffer.length == 0 && !handle.closed) {
        if (timeout_ms == 0) {
            [handle.cond wait];
        } else {
            NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:(double)timeout_ms / 1000.0];
            if (![handle.cond waitUntilDate:deadline]) {
                break;
            }
        }
    }

    if (handle.rxBuffer.length > 0) {
        size_t toCopy = handle.rxBuffer.length < cap ? handle.rxBuffer.length : cap;
        memcpy(buf, handle.rxBuffer.bytes, toCopy);
        [handle.rxBuffer replaceBytesInRange:NSMakeRange(0, toCopy) withBytes:NULL length:0];
        [handle.cond unlock];
        return (ssize_t)toCopy;
    }

    BOOL isClosed = handle.closed;
    [handle.cond unlock];
    return isClosed ? 0 : -2;
}

int murmur_rfcomm_write(void *ptr, const uint8_t *buf, size_t len) {
    if (!ptr || (!buf && len > 0)) {
        return -1;
    }
    if (len == 0) {
        return 0;
    }
    MurmurRfcommHandle *handle = (__bridge MurmurRfcommHandle *)ptr;
    NSData *payload = [NSData dataWithBytes:buf length:len];
    __block int status = 0;

    murmur_run_on_bt_thread(^{
        if (handle.closed || !handle.channel || ![handle.channel isOpen]) {
            status = -1;
            return;
        }
        BluetoothRFCOMMMTU mtu = [handle.channel getMTU];
        size_t chunkCap = mtu > 0 ? (size_t)mtu : 512;
        size_t offset = 0;
        const uint8_t *raw = (const uint8_t *)payload.bytes;
        while (offset < payload.length) {
            size_t rem = payload.length - offset;
            UInt16 n = (UInt16)(rem < chunkCap ? rem : chunkCap);
            IOReturn wr = [handle.channel writeSync:(void *)(raw + offset) length:n];
            if (wr != kIOReturnSuccess) {
                status = -1;
                return;
            }
            offset += n;
        }
    });

    return status;
}

int murmur_rfcomm_is_connected(void *ptr) {
    if (!ptr) {
        return 0;
    }
    MurmurRfcommHandle *handle = (__bridge MurmurRfcommHandle *)ptr;
    [handle.cond lock];
    BOOL closed = handle.closed;
    IOBluetoothRFCOMMChannel *chan = handle.channel;
    [handle.cond unlock];
    return (!closed && chan != nil && [chan isOpen]) ? 1 : 0;
}

void murmur_rfcomm_disconnect(void *ptr) {
    if (!ptr) {
        return;
    }
    MurmurRfcommHandle *handle = (__bridge MurmurRfcommHandle *)ptr;
    [handle.cond lock];
    handle.closed = YES;
    [handle.cond broadcast];
    [handle.cond unlock];

    murmur_run_on_bt_thread(^{
        if (handle.channel) {
            [handle.channel setDelegate:nil];
            [handle.channel closeChannel];
            handle.channel = nil;
        }
    });
}

void murmur_rfcomm_release(void *ptr) {
    if (!ptr) {
        return;
    }
    CFBridgingRelease(ptr);
}

// ---------------------------------------------------------------------------
// Classic L2CAP Transport Bridge (AirPods AAP PSM 0x1001)
// ---------------------------------------------------------------------------

@interface MurmurL2capHandle : NSObject <IOBluetoothL2CAPChannelDelegate>
@property (nonatomic, strong) IOBluetoothL2CAPChannel *channel;
@property (nonatomic, strong) NSCondition *cond;
@property (nonatomic, strong) NSMutableArray<NSData *> *rxPackets;
@property (nonatomic, assign) BOOL closed;
@end

@implementation MurmurL2capHandle

- (instancetype)init {
    self = [super init];
    if (self) {
        _cond = [[NSCondition alloc] init];
        _rxPackets = [[NSMutableArray alloc] init];
        _closed = NO;
    }
    return self;
}

- (void)l2capChannelData:(IOBluetoothL2CAPChannel *)l2capChannel
                    data:(void *)dataPointer
                  length:(size_t)dataLength {
    (void)l2capChannel;
    if (!dataPointer || dataLength == 0) {
        return;
    }
    NSData *pkt = [NSData dataWithBytes:dataPointer length:dataLength];
    [self.cond lock];
    if (!self.closed) {
        [self.rxPackets addObject:pkt];
    }
    [self.cond broadcast];
    [self.cond unlock];
}

- (void)l2capChannelClosed:(IOBluetoothL2CAPChannel *)l2capChannel {
    (void)l2capChannel;
    [self.cond lock];
    self.closed = YES;
    [self.cond broadcast];
    [self.cond unlock];
}

@end

void *murmur_l2cap_connect(
    const char *addr_utf8,
    uint16_t psm,
    char *err_buf,
    size_t err_len
) {
    __block void *result = NULL;
    murmur_run_on_bt_thread(^{
        IOBluetoothDevice *dev = murmur_find_device(addr_utf8);
        if (!dev) {
            murmur_set_err(err_buf, err_len, @"Bluetooth device not found in paired macOS devices");
            return;
        }

        MurmurL2capHandle *handle = [[MurmurL2capHandle alloc] init];
        IOBluetoothL2CAPChannel *chan = nil;
        IOReturn openStatus = [dev openL2CAPChannelSync:&chan
                                                withPSM:(BluetoothL2CAPPSM)psm
                                               delegate:handle];
        if (openStatus != kIOReturnSuccess || !chan) {
            murmur_set_err(
                err_buf,
                err_len,
                [NSString stringWithFormat:@"openL2CAPChannelSync(psm=0x%04x) failed (0x%x)", psm, openStatus]
            );
            return;
        }

        handle.channel = chan;
        result = (void *)CFBridgingRetain(handle);
    });
    return result;
}

ssize_t murmur_l2cap_read(void *ptr, uint8_t *buf, size_t cap, uint32_t timeout_ms) {
    if (!ptr || !buf || cap == 0) {
        return -1;
    }
    MurmurL2capHandle *handle = (__bridge MurmurL2capHandle *)ptr;
    [handle.cond lock];
    while (handle.rxPackets.count == 0 && !handle.closed) {
        if (timeout_ms == 0) {
            [handle.cond wait];
        } else {
            NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:(double)timeout_ms / 1000.0];
            if (![handle.cond waitUntilDate:deadline]) {
                break;
            }
        }
    }

    if (handle.rxPackets.count > 0) {
        NSData *pkt = handle.rxPackets.firstObject;
        [handle.rxPackets removeObjectAtIndex:0];
        size_t toCopy = pkt.length < cap ? pkt.length : cap;
        memcpy(buf, pkt.bytes, toCopy);
        [handle.cond unlock];
        return (ssize_t)toCopy;
    }

    BOOL isClosed = handle.closed;
    [handle.cond unlock];
    return isClosed ? 0 : -2;
}

int murmur_l2cap_write(void *ptr, const uint8_t *buf, size_t len) {
    if (!ptr || (!buf && len > 0)) {
        return -1;
    }
    if (len == 0) {
        return 0;
    }
    MurmurL2capHandle *handle = (__bridge MurmurL2capHandle *)ptr;
    NSData *payload = [NSData dataWithBytes:buf length:len];
    __block int status = 0;

    murmur_run_on_bt_thread(^{
        if (handle.closed || !handle.channel || ![handle.channel isOpen]) {
            status = -1;
            return;
        }
        BluetoothL2CAPMTU mtu = [handle.channel getOutgoingMTU];
        size_t chunkCap = mtu > 0 ? (size_t)mtu : 672;
        size_t offset = 0;
        const uint8_t *raw = (const uint8_t *)payload.bytes;
        while (offset < payload.length) {
            size_t rem = payload.length - offset;
            UInt16 n = (UInt16)(rem < chunkCap ? rem : chunkCap);
            IOReturn wr = [handle.channel writeSync:(void *)(raw + offset) length:n];
            if (wr != kIOReturnSuccess) {
                status = -1;
                return;
            }
            offset += n;
        }
    });

    return status;
}

int murmur_l2cap_is_connected(void *ptr) {
    if (!ptr) {
        return 0;
    }
    MurmurL2capHandle *handle = (__bridge MurmurL2capHandle *)ptr;
    [handle.cond lock];
    BOOL closed = handle.closed;
    IOBluetoothL2CAPChannel *chan = handle.channel;
    [handle.cond unlock];
    return (!closed && chan != nil && [chan isOpen]) ? 1 : 0;
}

void murmur_l2cap_disconnect(void *ptr) {
    if (!ptr) {
        return;
    }
    MurmurL2capHandle *handle = (__bridge MurmurL2capHandle *)ptr;
    [handle.cond lock];
    handle.closed = YES;
    [handle.cond broadcast];
    [handle.cond unlock];

    murmur_run_on_bt_thread(^{
        if (handle.channel) {
            [handle.channel setDelegate:nil];
            [handle.channel closeChannel];
            handle.channel = nil;
        }
    });
}

void murmur_l2cap_release(void *ptr) {
    if (!ptr) {
        return;
    }
    CFBridgingRelease(ptr);
}
