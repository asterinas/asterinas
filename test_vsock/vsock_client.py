import socket

client_socket = socket.socket(socket.AF_VSOCK, socket.SOCK_STREAM)
CID = socket.VMADDR_CID_HOST
PORT = 1234
vm_cid = 3   
server_port = 4321
client_socket.bind((CID, PORT))
client_socket.connect((vm_cid, server_port))

client_socket.sendall(b'Hello from host')

response = client_socket.recv(4096)
print(f'Received: {response.decode()}')

client_socket.close()
