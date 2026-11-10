fn idle(device: u64) {
    vkDeviceWaitIdle(device);
}

fn drop_device(renderer: Renderer) {
    renderer.abandon();
}
