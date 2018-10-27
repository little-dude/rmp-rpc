# This can be used to make sure connections are handled asynchronously by the
# server, in the example are example/server.rs.
#
# Run it with:
#
# ```
# python issue19.py & ; sleep 2 ; python issue19.py
# ```
#
# The expectation is that the second instance of this script should finish
# within 7 seconds.
#
# This example requires https://github.com/studio-ousia/mprpc
import os
import time
import mprpc


pid = os.getpid()
print(f'{pid}: {time.time()}: client connecting')
client = mprpc.RPCClient('127.0.0.1', 54321)
print(f'{pid}: {time.time()}: client connected, sending request')
client.call('do_long_computation', 5)
print(f'{pid}: {time.time()}: got response')
