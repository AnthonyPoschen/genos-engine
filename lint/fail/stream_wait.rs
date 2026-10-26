fn tick_materials() {
    wait_for_fences();
}

fn submit_transfer() {
    queue_wait_idle();
}

fn transfer_signaled() {
    wait_fences();
}
