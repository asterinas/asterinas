# Set a software breakpoint and stop in `hello_world`
set pagination off
break hello_world
run
continue

# Inspect the current stop
info breakpoints
backtrace
frame 0
print x
info registers rip rsp

# Modify the register and memory
set var x = 1000
print heap_value
x/wd heap_value
set {int}heap_value = 1234

# Single-step, remove the breakpoint, and finish execution
step
delete 1
continue
