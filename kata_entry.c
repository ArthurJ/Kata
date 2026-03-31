
extern void main();
extern void kata_rt_boot(void (*main_action)());

int main(int argc, char** argv) {
    kata_rt_boot(main);
    return 0;
}
