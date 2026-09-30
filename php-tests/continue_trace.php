#!/usr/bin/env php
<?php
/**
 * Continue-from-HTTP_TRACEPARENT fixture for xtask.
 * RINIT should attach the SERVER root to the inbound TraceID.
 */
if (!extension_loaded('otel_auto')) {
    fwrite(STDERR, "otel_auto not loaded\n");
    exit(1);
}
echo "continue_ok\n";
