# In this script, a single client makes 1000 asynchronous requests to the
# server implemented in examples/server.rs
import time
import msgpackrpc

client = client = msgpackrpc.Client(msgpackrpc.Address("127.0.0.1", 54321))
start = time.time()

requests = []
for i in range(0, 1000):
    requests.append(client.call_async('do_long_computation', 5))

for req in requests:
    req.get()

end = time.time()
print(end - start)
