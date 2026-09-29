#!/usr/bin/env php
<?php
/**
 * Integration harness: exercise hooks when extensions exist; always emits a root span via request lifecycle.
 *
 * Stub Grpc\BaseStub so observer can fire without pecl-grpc.
 */

declare(strict_types=1);

if (!extension_loaded('otel_auto')) {
    fwrite(STDERR, "FAIL: otel_auto not loaded\n");
    exit(1);
}

// Minimal stand-in for Grpc\BaseStub::_simpleRequest first arg path parsing.
if (!class_exists('Grpc\\BaseStub', false)) {
    eval(<<<'PHP'
namespace Grpc;
class BaseStub {
    public function _simpleRequest($path, $argument, $deserialize, array $metadata = [], array $options = []) {
        return (object)['path' => $path, 'metadata' => $metadata];
    }
}
PHP);
}

$stub = new Grpc\BaseStub();
$stub->_simpleRequest(
    '/LinkServiceProto.LinkService/getTargetLink',
    null,
    null,
    [],
    []
);

// PDO if available
if (extension_loaded('pdo') && extension_loaded('pdo_sqlite')) {
    $pdo = new PDO('sqlite::memory:');
    $pdo->exec('CREATE TABLE t (id INTEGER)');
    $pdo->query('SELECT 1');
    $st = $pdo->prepare('SELECT id FROM t WHERE id = ?');
    $st->execute([1]);
}

// Redis if available
if (class_exists('Redis')) {
    try {
        $r = new Redis();
        // May fail to connect; still exercises observer begin/end.
        @$r->connect('127.0.0.1', 6379, 0.2);
        @$r->set('otel_test', '1');
        @$r->get('otel_test');
    } catch (Throwable $e) {
        // ignore
    }
}

// curl if available
if (function_exists('curl_exec')) {
    $ch = curl_init('http://127.0.0.1:9/');
    curl_setopt($ch, CURLOPT_RETURNTRANSFER, true);
    curl_setopt($ch, CURLOPT_TIMEOUT_MS, 50);
    @curl_exec($ch);
    curl_close($ch);
}

echo "integration hooks exercised\n";
