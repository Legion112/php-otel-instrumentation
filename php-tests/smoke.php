#!/usr/bin/env php
<?php
/**
 * Smoke script for integration tests (run inside PHP with otel_auto loaded).
 * Emits synthetic activity; extension should export spans if OTEL_ENDPOINT is set.
 */
echo "php=" . PHP_VERSION . PHP_EOL;
echo "modules=" . implode(',', get_loaded_extensions()) . PHP_EOL;
if (!extension_loaded('otel_auto')) {
    fwrite(STDERR, "otel_auto not loaded\n");
    exit(1);
}
echo "otel_auto loaded\n";

// PDO / redis / curl / grpc may be absent in bare image; just prove load + RINIT/RSHUTDOWN.
echo "ok\n";
