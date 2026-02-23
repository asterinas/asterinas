#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <grp.h>
#include <limits.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <stdarg.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>


static int g_debug = 0;


// 错误处理和工具函数

static void die(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    vfprintf(stderr, fmt, ap);
    va_end(ap);
    fputc('\n', stderr);
    exit(1);
}

static void xclose(int fd) {
    if (fd >= 0) close(fd);
}

static void ensure_dir(const char *p, mode_t m) {
    if (mkdir(p, m) == 0) return;
    if (errno == EEXIST) return;
    die("mkdir(%s) errno=%d(%s)", p, errno, strerror(errno));
}

static void join(char *out, size_t n, const char *a, const char *b) {
    snprintf(out, n, "%s/%s", a, b);
}

// 枚举类型定义

typedef enum { REL_OWNER=0, REL_GROUP=1, REL_OTHER=2 } Rel;
static const char *rel_str(Rel r) {
    return r==REL_OWNER ? "OWNER" : (r==REL_GROUP ? "GROUP" : "OTHER");
}

typedef enum { OP_OPEN=0, OP_OPENAT=1 } Op;
static const char *op_str(Op o) { return o==OP_OPEN ? "open" : "openat"; }

typedef enum { ACC_R=0, ACC_W=1, ACC_RW=2 } Acc;
static const char *acc_str(Acc a) { return a==ACC_R ? "R" : (a==ACC_W ? "W" : "RW"); }

typedef enum { T_REG=0, T_DIR=1 } TargetType;
static const char *tt_str(TargetType t) { return t==T_REG ? "REG" : "DIR"; }

typedef enum { SYM_NONE=0, SYM_1=1, SYM_LOOP=2 } SymMode;
static const char *sym_str(SymMode s) { return s==SYM_NONE ? "NONE" : (s==SYM_1 ? "SYM1" : "LOOP"); }

// 用户/组ID常量


static const uid_t U_SUBJ = 1000;
static const gid_t G_SUBJ = 1000;
static const uid_t U_NOT  = 2000;
static const gid_t G_FILE = 2222;
static const gid_t G_NOT  = 3333;

// 权限位计算


static mode_t mode_for_class(Rel rel, bool r, bool w, bool x) {
    mode_t bits = 0;
    if (r) bits |= 4;
    if (w) bits |= 2;
    if (x) bits |= 1;
    if (rel == REL_OWNER) return (bits << 6);
    if (rel == REL_GROUP) return (bits << 3);
    return bits;
}

// 身份切换


static int set_subject_ids(int sg_set) {
    if (setgroups(0, NULL) < 0) {
        die("setgroups(clear) errno=%d(%s)", errno, strerror(errno));
    }
    if (sg_set) {
        gid_t groups[1] = { G_FILE };
        if (setgroups(1, groups) < 0) 
            die("setgroups([%d]) errno=%d(%s)", (int)G_FILE, errno, strerror(errno));
    }
    if (setegid(G_SUBJ) < 0) 
        die("setegid(%d) errno=%d(%s)", (int)G_SUBJ, errno, strerror(errno));
    if (seteuid(U_SUBJ) < 0) 
        die("seteuid(%d) errno=%d(%s)", (int)U_SUBJ, errno, strerror(errno));
    return 0;
}

static void restore_root_ids(void) {
    if (setegid(0) < 0) 
        die("restore_root(setegid) errno=%d(%s)", errno, strerror(errno));
    if (seteuid(0) < 0) 
        die("restore_root(seteuid) errno=%d(%s)", errno, strerror(errno));
    if (setgroups(0, NULL) < 0) 
        die("restore_root(setgroups) errno=%d(%s)", errno, strerror(errno));
}


// Tree结构体


typedef struct {
    char base[PATH_MAX];
    char one[PATH_MAX];
    char two[PATH_MAX];
    char one_d1[PATH_MAX];
    char two_d1[PATH_MAX];
    char two_d2[PATH_MAX];
    
    char objf_one[PATH_MAX];
    char objd_one[PATH_MAX];
    char lnk_one[PATH_MAX];
    char loop_one[PATH_MAX];
    
    char objf_two[PATH_MAX];
    char objd_two[PATH_MAX];
    char lnk_two[PATH_MAX];
    char loop_two[PATH_MAX];
    
    int base_fd;
    int one_fd;
    int two_fd;
    int one_d1_fd;
    int two_d1_fd;
    int two_d2_fd;
} Tree;

static void init_tree(Tree *t, const char *base) {
    memset(t, 0, sizeof(*t));
    snprintf(t->base, sizeof(t->base), "%s", base);
    
    join(t->one, sizeof(t->one), t->base, "one");
    join(t->two, sizeof(t->two), t->base, "two");
    join(t->one_d1, sizeof(t->one_d1), t->one, "d1");
    join(t->two_d1, sizeof(t->two_d1), t->two, "d1");
    join(t->two_d2, sizeof(t->two_d2), t->two_d1, "d2");
    
    join(t->objf_one, sizeof(t->objf_one), t->one_d1, "objf");
    join(t->objd_one, sizeof(t->objd_one), t->one_d1, "objd");
    join(t->lnk_one,  sizeof(t->lnk_one),  t->one_d1, "lnk");
    join(t->loop_one, sizeof(t->loop_one), t->one_d1, "loop");
    
    join(t->objf_two, sizeof(t->objf_two), t->two_d2, "objf");
    join(t->objd_two, sizeof(t->objd_two), t->two_d2, "objd");
    join(t->lnk_two,  sizeof(t->lnk_two),  t->two_d2, "lnk");
    join(t->loop_two, sizeof(t->loop_two), t->two_d2, "loop");
    
    t->base_fd = t->one_fd = t->two_fd = -1;
    t->one_d1_fd = t->two_d1_fd = t->two_d2_fd = -1;
}

static void create_base_dirs(const Tree *t) {
    ensure_dir(t->base, 0755);
    ensure_dir(t->one, 0755);
    ensure_dir(t->two, 0755);
    ensure_dir(t->one_d1, 0755);
    ensure_dir(t->two_d1, 0755);
    ensure_dir(t->two_d2, 0755);
}

static void open_persistent_fds(Tree *t) {
    t->base_fd = open(t->base, O_RDONLY|O_DIRECTORY);
    t->one_fd = open(t->one, O_RDONLY|O_DIRECTORY);
    t->two_fd = open(t->two, O_RDONLY|O_DIRECTORY);
    t->one_d1_fd = open(t->one_d1, O_RDONLY|O_DIRECTORY);
    t->two_d1_fd = open(t->two_d1, O_RDONLY|O_DIRECTORY);
    t->two_d2_fd = open(t->two_d2, O_RDONLY|O_DIRECTORY);
    
    if (t->base_fd < 0 || t->one_fd < 0 || t->two_fd < 0 ||
        t->one_d1_fd < 0 || t->two_d1_fd < 0 || t->two_d2_fd < 0) {
        die("Failed to open persistent directory FDs");
    }
}

static void close_persistent_fds(Tree *t) {
    xclose(t->base_fd);
    xclose(t->one_fd);
    xclose(t->two_fd);
    xclose(t->one_d1_fd);
    xclose(t->two_d1_fd);
    xclose(t->two_d2_fd);
}

static void restore_all_perms(Tree *t) {
    fchmod(t->base_fd, 0755);
    fchmod(t->one_fd, 0755);
    fchmod(t->two_fd, 0755);
    fchmod(t->one_d1_fd, 0755);
    fchmod(t->two_d1_fd, 0755);
    fchmod(t->two_d2_fd, 0755);
}

static void unlinkat_safe(int dirfd, const char *name) {
    unlinkat(dirfd, name, 0);
    unlinkat(dirfd, name, AT_REMOVEDIR);
}

static void config_objects(Tree *t, int pathdepth, TargetType tt, int exists) {
    int leaffd = (pathdepth==1) ? t->one_d1_fd : t->two_d2_fd;
    const char *objf_name = "objf";
    const char *objd_name = "objd";
    
    restore_all_perms(t);
    
    unlinkat_safe(leaffd, objf_name);
    unlinkat_safe(leaffd, objd_name);
    
    if (!exists) return;
    
    if (tt == T_REG) {
        int fd = openat(leaffd, objf_name, O_CREAT|O_WRONLY|O_TRUNC, 0644);
        if (fd >= 0) {
            write(fd, "x", 1);
            close(fd);
        }
    } else {
        mkdirat(leaffd, objd_name, 0755);
    }
}

static void config_links(Tree *t, int pathdepth, TargetType tt, SymMode sym) {
    int leaffd = (pathdepth==1) ? t->one_d1_fd : t->two_d2_fd;
    
    unlinkat_safe(leaffd, "lnk");
    unlinkat_safe(leaffd, "loop");
    
    if (sym == SYM_1) {
        const char *target = (tt==T_REG) ? "objf" : "objd";
        symlinkat(target, leaffd, "lnk");
    } else if (sym == SYM_LOOP) {
        symlinkat("loop", leaffd, "loop");
    }
}

static void apply_dir_perms(Tree *t, int pathdepth, Rel dirrel, int r, int w, int x) {
    int leaffd = (pathdepth==1) ? t->one_d1_fd : t->two_d2_fd;
    const char *leafpath = (pathdepth==1) ? t->one_d1 : t->two_d2;
    
    restore_all_perms(t);
    
    uid_t u = (dirrel==REL_OWNER) ? U_SUBJ : U_NOT;
    gid_t g = (dirrel==REL_GROUP) ? G_FILE : G_NOT;
    mode_t m = mode_for_class(dirrel, r, w, x);
    
    lchown(leafpath, u, g);
    fchmod(leaffd, m);
}

static void apply_target_perms(Tree *t, int pathdepth, TargetType tt, Rel filerel, int r, int w) {
    int leaffd = (pathdepth==1) ? t->one_d1_fd : t->two_d2_fd;
    const char *name = (tt==T_REG) ? "objf" : "objd";
    
    restore_all_perms(t);
    
    uid_t u = (filerel==REL_OWNER) ? U_SUBJ : U_NOT;
    gid_t g = (filerel==REL_GROUP) ? G_FILE : G_NOT;
    mode_t m = mode_for_class(filerel, r, w, 0);
    if (tt==T_DIR) m |= mode_for_class(filerel, r, w, 1);
    
    struct stat st;
    if (fstatat(leaffd, name, &st, AT_SYMLINK_NOFOLLOW) == 0) {
        fchownat(leaffd, name, u, g, AT_SYMLINK_NOFOLLOW);
        fchmodat(leaffd, name, m, 0);
    }
}


// 标志位生成（17个标志位）

static int flags_from_dims(Acc acc, 
                          int creat, int excl, int trunc, int append,
                          int nofollow, int odir, int cloexec,
                          int o_path, int o_tmpfile, int o_noatime,
                          int o_noctty, int o_nonblock, int o_direct,
                          int o_sync, int o_dsync, int o_async) {
    int flags = 0;
    
    // 访问模式
    if (acc == ACC_R) flags |= O_RDONLY;
    else if (acc == ACC_W) flags |= O_WRONLY;
    else flags |= O_RDWR;
    
    // 文件创建标志
    if (creat) flags |= O_CREAT;
    if (excl) flags |= O_EXCL;
    if (o_noctty) flags |= O_NOCTTY;
    if (trunc) flags |= O_TRUNC;
    
    // 文件状态标志
    if (append) flags |= O_APPEND;
    if (o_async) flags |= O_ASYNC;
    if (o_direct) flags |= O_DIRECT;
    if (o_dsync) flags |= O_DSYNC;
    if (o_noatime) flags |= O_NOATIME;
    if (o_nonblock) flags |= O_NONBLOCK;
    if (o_sync) flags |= O_SYNC;
    
    // 其他标志
    if (cloexec) flags |= O_CLOEXEC;
    if (odir) flags |= O_DIRECTORY;
    if (nofollow) flags |= O_NOFOLLOW;
    if (o_path) flags |= O_PATH;
    if (o_tmpfile) flags |= O_TMPFILE;
    
    return flags;
}


// Case结构（完整版 - 包含所有标志位）


typedef struct {
    uint64_t id;
    Op op;
    Acc acc;
    int pathdepth;
    int dotdot;
    int trail;
    TargetType tt;
    SymMode sym;
    Rel dir_rel;
    Rel file_rel;
    int dir_r, dir_w, dir_x;
    int file_r, file_w;
    
    // 文件创建标志
    int creat;
    int excl;
    int o_noctty;
    int trunc;
    
    // 文件状态标志
    int append;
    int o_async;
    int o_direct;
    int o_dsync;
    int o_noatime;
    int o_nonblock;
    int o_sync;
    
    // 其他标志
    int nofollow;
    int o_directory;
    int cloexec;
    int o_path;
    int o_tmpfile;
    
    int exists;
    int sg_req;
    int sg_set;
} Case;


// 约束检查函数 （24条规则）


static bool is_valid_case(const Case *c) {
    
    // 规则1: O_TRUNC只对写模式有意义
    if (c->trunc && c->acc == ACC_R) {
        return false;
    }
    
    // 规则2: O_APPEND只对写模式有意义
    if (c->append && c->acc == ACC_R) {
        return false;
    }
    
    // 规则3: O_DIRECTORY不能用于普通文件
    if (c->o_directory && c->tt == T_REG) {
        return false;
    }
    
    // 规则4: O_EXCL必须与O_CREAT一起使用
    if (c->excl && !c->creat) {
        return false;
    }
    
    // 规则5: O_NOFOLLOW在没有符号链接时没意义
    if (c->nofollow && c->sym == SYM_NONE) {
        return false;
    }
    
    // 规则6: 文件不存在时，文件权限无意义
    if (!c->exists) {
        if (c->file_r != 0 || c->file_w != 0) {
            return false;
        }
    }
    
    // 规则7: 目录无执行权限时，文件权限无关紧要
    if (!c->dir_x && c->exists) {
        if (c->file_r != 0 || c->file_w != 0) {
            return false;
        }
    }
    
    // 规则8: sg_set只在sg_req=1时有意义
    if (!c->sg_req && c->sg_set) {
        return false;
    }
    
    // 规则9: 循环链接时只测试T_REG
    if (c->sym == SYM_LOOP && c->tt != T_REG) {
        return false;
    }
    
    // 规则10: dotdot只在openat时测试
    if (c->dotdot && c->op == OP_OPEN) {
        return false;
    }
    
    // 规则11: trail主要影响目录
    if (c->trail && c->tt == T_REG && c->sym == SYM_NONE) {
        return false;
    }
    
    // 规则12: O_CREAT与exists的组合优化
    if (c->creat && c->exists) {
        if (!c->dir_w) {
            return false;
        }
    }
    
    // 规则13: 权限组合优化 - 避免全0权限的冗余测试
    if (!c->dir_r && !c->dir_w && !c->dir_x && 
        !c->file_r && !c->file_w) {
        if (c->creat || c->trunc || c->append) {
            return false;
        }
    }
    
    // 规则14: pathdepth优化
    if (c->pathdepth == 2) {
        if (c->dir_r && c->dir_w && c->dir_x) {
            return false;
        }
    }
    
    //  O_PATH (规则15) ==========
    
    if (c->o_path) {
        // O_PATH只能与O_RDONLY配合
        if (c->acc != ACC_R) {
            return false;
        }
        
        // O_PATH不能与这些标志位组合
        if (c->creat || c->excl || c->trunc || c->append) {
            return false;
        }
        
        // O_PATH与O_DIRECTORY冗余
        if (c->o_directory) {
            return false;
        }
        
        // O_PATH不需要文件权限测试
        if (c->file_r != 0 || c->file_w != 0) {
            return false;
        }
        
        // O_PATH + 循环链接简化
        if (c->sym == SYM_LOOP) {
            return false;
        }
        
        // O_PATH与其他特殊标志互斥
        if (c->o_tmpfile || c->o_noatime || c->o_direct) {
            return false;
        }
    }
    
    //规则16: O_TMPFILE
    
    if (c->o_tmpfile) {
        // O_TMPFILE必须用于目录
        if (c->tt != T_DIR) {
            return false;
        }
        
        // O_TMPFILE必须与O_CREAT配合
        if (!c->creat) {
            return false;
        }
        
        // 临时文件总是新建的
        if (c->exists) {
            return false;
        }
        
        // 不能与符号链接组合
        if (c->sym != SYM_NONE) {
            return false;
        }
        
        // 需要目录写权限
        if (!c->dir_w) {
            return false;
        }
        
        // O_TMPFILE + O_EXCL简化
        if (c->excl) {
            return false;
        }
        
        // 只测试写模式
        if (c->acc == ACC_R) {
            return false;
        }
        
        // O_TMPFILE与某些标志位冲突
        if (c->o_directory || c->nofollow) {
            return false;
        }
    }
    
    // 规则17: O_NOATIME
    
    if (c->o_noatime) {
        // 只在owner关系时测试
        if (c->file_rel != REL_OWNER) {
            return false;
        }
        
        // 文件不存在时无意义
        if (!c->exists) {
            return false;
        }
        
        // 对目录意义不大
        if (c->tt == T_DIR) {
            return false;
        }
        
        // 主要影响读操作
        if (c->acc == ACC_W) {
            return false;
        }
    }
    
    //规则18: O_NOCTTY
    
    if (c->o_noctty) {
        // O_NOCTTY对常规文件影响很小，简化测试
        // 只在少数组合下测试
        if (c->pathdepth == 2 || c->dotdot) {
            return false;
        }
        
        // 主要与设备文件相关，对目录无意义
        if (c->tt == T_DIR) {
            return false;
        }
    }
    
    //规则19: O_NONBLOCK 
    
    if (c->o_nonblock) {
        // O_NONBLOCK对常规文件几乎无影响
        // 只在基本场景下测试
        if (c->pathdepth == 2 && c->dotdot) {
            return false;
        }
        
        // 主要影响FIFO和设备，简化常规文件测试
        if (c->tt == T_REG && c->sym != SYM_NONE) {
            return false;
        }
        
        // 与O_PATH冲突（O_PATH返回的FD不能用于I/O）
        if (c->o_path) {
            return false;
        }
    }
    
    // 规则20: O_DIRECT
    
    if (c->o_direct) {
        // O_DIRECT对目录无效
        if (c->tt == T_DIR) {
            return false;
        }
        
        // O_DIRECT + O_PATH无意义
        if (c->o_path) {
            return false;
        }
        
        // 简化：只在写模式下测试
        if (c->acc == ACC_R) {
            return false;
        }
        
        // 文件必须存在或正在创建
        if (!c->exists && !c->creat) {
            return false;
        }
    }
    
    
    // 规则21: O_SYNC/O_DSYNC

    // O_SYNC和O_DSYNC不能同时设置
    if (c->o_sync && c->o_dsync) {
        return false;
    }
    
    if (c->o_sync || c->o_dsync) {
        // 同步标志只对写操作有意义
        if (c->acc == ACC_R) {
            return false;
        }
        
        // 对目录无意义
        if (c->tt == T_DIR) {
            return false;
        }
        
        // 与O_PATH冲突
        if (c->o_path) {
            return false;
        }
        
        // 简化测试：只在基本场景测试
        if (c->pathdepth == 2 || c->sym != SYM_NONE) {
            return false;
        }
    }
    
    //规则22: O_ASYNC
    
    if (c->o_async) {
        // O_ASYNC主要用于信号驱动I/O，对open返回值影响很小
        // 大幅简化测试
        if (c->pathdepth == 2 || c->dotdot || c->trail) {
            return false;
        }
        
        // 只在文件上测试
        if (c->tt == T_DIR) {
            return false;
        }
        
        // 与O_PATH冲突
        if (c->o_path) {
            return false;
        }
        
        // 简化：只测试基本权限组合
        if (!c->dir_x || !c->file_r) {
            return false;
        }
    }
    
    //规则23: 写相关标志位组合
    
    // O_TRUNC和O_APPEND通常不同时使用（语义冲突）
    if (c->trunc && c->append) {
        // 虽然技术上允许，但简化测试
        return false;
    }
    
    //规则24: 特殊组合优化
    
    // 多个同步标志不要同时测试（边际价值低）
    int sync_count = c->o_sync + c->o_dsync + c->o_direct;
    if (sync_count > 1) {
        return false;
    }
    
    // 高级I/O标志（DIRECT/ASYNC/SYNC）不要与复杂路径组合
    if ((c->o_direct || c->o_async || c->o_sync || c->o_dsync) &&
        (c->pathdepth == 2 || c->sym != SYM_NONE)) {
        return false;
    }
    
    return true;
}

// CSV输出（完整版）


static void emit_header(FILE *fp) {
    fprintf(fp,
        "id,op,acc,pathdepth,dotdot,trail,target_type,sym_mode,dir_rel,file_rel,"
        "dir_r,dir_w,dir_x,file_r,file_w,"
        "creat,excl,o_noctty,trunc,"
        "append,o_async,o_direct,o_dsync,o_noatime,o_nonblock,o_sync,"
        "nofollow,o_directory,cloexec,o_path,o_tmpfile,"
        "exists,sg_req,sg_set,real_ok,real_errno\n");
}

static void emit_row(FILE *fp, const Case *c, int ok, int err) {
    fprintf(fp,
        "%llu,%s,%s,%d,%d,%d,%s,%s,%s,%s,"
        "%d,%d,%d,%d,%d,"
        "%d,%d,%d,%d,"
        "%d,%d,%d,%d,%d,%d,%d,"
        "%d,%d,%d,%d,%d,"
        "%d,%d,%d,%d,%d\n",
        (unsigned long long)c->id,
        op_str(c->op), acc_str(c->acc),
        c->pathdepth, c->dotdot, c->trail,
        tt_str(c->tt), sym_str(c->sym),
        rel_str(c->dir_rel), rel_str(c->file_rel),
        c->dir_r, c->dir_w, c->dir_x,
        c->file_r, c->file_w,
        c->creat, c->excl, c->o_noctty, c->trunc,
        c->append, c->o_async, c->o_direct, c->o_dsync, 
        c->o_noatime, c->o_nonblock, c->o_sync,
        c->nofollow, c->o_directory, c->cloexec, 
        c->o_path, c->o_tmpfile,
        c->exists, c->sg_req, c->sg_set,
        ok, err
    );
}

// 执行测试用例


static void run_case(Tree *t, const Case *c, int *ok_out, int *err_out) {
    config_objects(t, c->pathdepth, c->tt, c->exists);
    config_links(t, c->pathdepth, c->tt, c->sym);
    apply_dir_perms(t, c->pathdepth, c->dir_rel, c->dir_r, c->dir_w, c->dir_x);
    apply_target_perms(t, c->pathdepth, c->tt, c->file_rel, c->file_r, c->file_w);
    
    const char *name;
    if (c->sym == SYM_NONE) name = (c->tt==T_REG) ? "objf" : "objd";
    else if (c->sym == SYM_1) name = "lnk";
    else name = "loop";
    
    char path_abs[PATH_MAX];
    char path_rel[PATH_MAX];
    
    if (c->pathdepth==1) {
        snprintf(path_abs, sizeof(path_abs), "%s/d1/%s", t->one, name);
    } else {
        snprintf(path_abs, sizeof(path_abs), "%s/d1/d2/%s", t->two, name);
    }
    
    if (!c->dotdot) {
        snprintf(path_rel, sizeof(path_rel), "%s", name);
    } else {
        const char *leafname = (c->pathdepth==1) ? "d1" : "d2";
        snprintf(path_rel, sizeof(path_rel), "../%s/%s", leafname, name);
    }
    
    if (c->trail) {
        size_t la = strlen(path_abs);
        if (la+1 < sizeof(path_abs)) { path_abs[la] = '/'; path_abs[la+1] = '\0'; }
        size_t lr = strlen(path_rel);
        if (lr+1 < sizeof(path_rel)) { path_rel[lr] = '/'; path_rel[lr+1] = '\0'; }
    }
    
    int leaffd = (c->pathdepth==1) ? t->one_d1_fd : t->two_d2_fd;
    
    set_subject_ids(c->sg_set);
    
    int flags = flags_from_dims(c->acc, 
                                c->creat, c->excl, c->trunc, c->append,
                                c->nofollow, c->o_directory, c->cloexec,
                                c->o_path, c->o_tmpfile, c->o_noatime,
                                c->o_noctty, c->o_nonblock, c->o_direct,
                                c->o_sync, c->o_dsync, c->o_async);
    errno = 0;
    int fd = -1;
    
    if (c->op == OP_OPEN) {
        fd = open(path_abs, flags, 0644);
    } else {
        fd = openat(leaffd, path_rel, flags, 0644);
    }
    
    if (fd >= 0) {
        *ok_out = 1;
        *err_out = 0;
        close(fd);
    } else {
        *ok_out = 0;
        *err_out = errno;
    }
    
    restore_root_ids();
}

// 测试用例遍历（17个标志位）


static uint64_t g_id = 0;
static uint64_t g_skipped = 0;

static void foreach_cases(Tree *t, FILE *out, uint64_t max_rows, uint64_t progress) {
    uint64_t emitted = 0;
    uint64_t total_checked = 0;
    
    fprintf(stderr, "\n[info] Starting Ultimate Complete Test Generator\n");
    fprintf(stderr, "[info] Total flags: 17 (excluding access modes)\n");
    fprintf(stderr, "[info] Theoretical combinations: ~191 billion\n");
    fprintf(stderr, "[info] Expected valid cases: ~80-100万\n");
    fprintf(stderr, "[info] Constraint rules: 24\n\n");
    
    Op ops[] = { OP_OPEN, OP_OPENAT };
    Acc accs[] = { ACC_R, ACC_W, ACC_RW };
    int pathdepths[] = { 1, 2 };
    int dotdots[] = { 0, 1 };
    int trails[] = { 0, 1 };
    TargetType tts[] = { T_REG, T_DIR };
    SymMode syms[] = { SYM_NONE, SYM_1, SYM_LOOP };
    Rel rels[] = { REL_OWNER, REL_GROUP, REL_OTHER };
    int bits[] = { 0, 1 };
    
    for (size_t iop=0; iop<2; iop++) {
    for (size_t iacc=0; iacc<3; iacc++) {
    for (size_t ipd=0; ipd<2; ipd++) {
    for (size_t idd=0; idd<2; idd++) {
    for (size_t itr=0; itr<2; itr++) {
    for (size_t itt=0; itt<2; itt++) {
    for (size_t isym=0; isym<3; isym++) {
    for (size_t idirrel=0; idirrel<3; idirrel++) {
    for (size_t ifilerel=0; ifilerel<3; ifilerel++) {
    for (size_t idir_r=0; idir_r<2; idir_r++) {
    for (size_t idir_w=0; idir_w<2; idir_w++) {
    for (size_t idir_x=0; idir_x<2; idir_x++) {
    for (size_t ifile_r=0; ifile_r<2; ifile_r++) {
    for (size_t ifile_w=0; ifile_w<2; ifile_w++) {
    for (size_t icreat=0; icreat<2; icreat++) {
    for (size_t iexcl=0; iexcl<2; iexcl++) {
    for (size_t inoctty=0; inoctty<2; inoctty++) {
    for (size_t itrunc=0; itrunc<2; itrunc++) {
    for (size_t iappend=0; iappend<2; iappend++) {
    for (size_t iasync=0; iasync<2; iasync++) {
    for (size_t idirect=0; idirect<2; idirect++) {
    for (size_t idsync=0; idsync<2; idsync++) {
    for (size_t inoatime=0; inoatime<2; inoatime++) {
    for (size_t inonblock=0; inonblock<2; inonblock++) {
    for (size_t isync=0; isync<2; isync++) {
    for (size_t inof=0; inof<2; inof++) {
    for (size_t iodir=0; iodir<2; iodir++) {
    for (size_t icloexec=0; icloexec<2; icloexec++) {
    for (size_t ipath=0; ipath<2; ipath++) {
    for (size_t itmpfile=0; itmpfile<2; itmpfile++) {
    for (size_t iex=0; iex<2; iex++) {
    for (size_t isg=0; isg<2; isg++) {
        
        total_checked++;
        
        Case c = {0};
        c.id = g_id;
        c.op = ops[iop];
        c.acc = accs[iacc];
        c.pathdepth = pathdepths[ipd];
        c.dotdot = dotdots[idd];
        c.trail = trails[itr];
        c.tt = tts[itt];
        c.sym = syms[isym];
        c.dir_rel = rels[idirrel];
        c.file_rel = rels[ifilerel];
        c.dir_r = bits[idir_r];
        c.dir_w = bits[idir_w];
        c.dir_x = bits[idir_x];
        c.file_r = bits[ifile_r];
        c.file_w = bits[ifile_w];
        c.creat = bits[icreat];
        c.excl = bits[iexcl];
        c.o_noctty = bits[inoctty];
        c.trunc = bits[itrunc];
        c.append = bits[iappend];
        c.o_async = bits[iasync];
        c.o_direct = bits[idirect];
        c.o_dsync = bits[idsync];
        c.o_noatime = bits[inoatime];
        c.o_nonblock = bits[inonblock];
        c.o_sync = bits[isync];
        c.nofollow = bits[inof];
        c.o_directory = bits[iodir];
        c.cloexec = bits[icloexec];
        c.o_path = bits[ipath];
        c.o_tmpfile = bits[itmpfile];
        c.exists = bits[iex];
        c.sg_set = bits[isg];
        c.sg_req = (c.dir_rel==REL_GROUP || c.file_rel==REL_GROUP) ? 1 : 0;
        
        // 关键优化：过滤无效组合
        if (!is_valid_case(&c)) {
            g_skipped++;
            continue;
        }
        
        g_id++;
        
        if (max_rows > 0 && emitted >= max_rows) {
            goto done;
        }
        
        int ok=0, err=0;
        run_case(t, &c, &ok, &err);
        emit_row(out, &c, ok, err);
        emitted++;
        
if (progress > 0 && (emitted % progress) == 0) {
    unsigned __int128 num = (unsigned __int128)g_skipped * 1000; // 百分比 *10
    uint64_t rate_x10 = total_checked ? (uint64_t)(num / total_checked) : 0;

    fprintf(stderr,
        "[progress] emitted=%llu skipped=%llu total=%llu filter_rate=%llu.%01llu%% last_id=%llu\n",
        (unsigned long long)emitted,
        (unsigned long long)g_skipped,
        (unsigned long long)total_checked,
        (unsigned long long)(rate_x10 / 10),
        (unsigned long long)(rate_x10 % 10),
        (unsigned long long)c.id);
}


    }}}}}}}}}}}}}}}}}}}}}}}}}}}}}}}}
    
done:
    fprintf(stderr, "\n[summary] ═══════════════════════════════════════\n");
    fprintf(stderr, "[summary] Total checked:  %llu\n", (unsigned long long)total_checked);
    fprintf(stderr, "[summary] Valid emitted:  %llu\n", (unsigned long long)emitted);
    fprintf(stderr, "[summary] Filtered out:   %llu\n", (unsigned long long)g_skipped);
    {
    unsigned __int128 num = (unsigned __int128)g_skipped * 10000; // 百分比 *100
    uint64_t rate_x100 = total_checked ? (uint64_t)(num / total_checked) : 0;
    fprintf(stderr, "[summary] Filter rate:    %llu.%02llu%%\n",
            (unsigned long long)(rate_x100 / 100),
            (unsigned long long)(rate_x100 % 100));
}

    fprintf(stderr, "[summary] ═══════════════════════════════════════\n");
}

// Main函数

static void usage(const char *argv0) {
    fprintf(stderr, "Ultimate Complete Test Generator - 17 flags\n\n");
    fprintf(stderr, "usage: %s --base <dir> --out <csv> [--max N] [--progress N]\n", argv0);
    fprintf(stderr, "\nOptions:\n");
    fprintf(stderr, "  --base <dir>     Base directory for test tree\n");
    fprintf(stderr, "  --out <csv>      Output CSV file\n");
    fprintf(stderr, "  --max N          Maximum rows to generate (0=unlimited)\n");
    fprintf(stderr, "  --progress N     Show progress every N rows\n");
    fprintf(stderr, "\nExample:\n");
    fprintf(stderr, "  sudo %s --base /tmp/test --out results.csv --progress 10000\n", argv0);
    exit(2);
}

int main(int argc, char **argv) {
    const char *base = NULL;
    const char *outp = NULL;
    uint64_t max_rows = 0;
    uint64_t progress = 0;
    
    for (int i=1; i<argc; i++) {
        if (!strcmp(argv[i], "--base") && i+1<argc) { base = argv[++i]; continue; }
        if (!strcmp(argv[i], "--out") && i+1<argc) { outp = argv[++i]; continue; }
        if (!strcmp(argv[i], "--max") && i+1<argc) { max_rows = strtoull(argv[++i], NULL, 10); continue; }
        if (!strcmp(argv[i], "--progress") && i+1<argc) { progress = strtoull(argv[++i], NULL, 10); continue; }
        usage(argv[0]);
    }
    if (!base || !outp) usage(argv[0]);
    
    if (geteuid() != 0) die("must run as root");
    
    fprintf(stderr, "\n");
    fprintf(stderr, "╔════════════════════════════════════════════════════════╗\n");
    fprintf(stderr, "║   Ultimate Complete Test Generator                    ║\n");
    fprintf(stderr, "║   Coverage: 17 flags (100%% of relevant flags)         ║\n");
    fprintf(stderr, "╚════════════════════════════════════════════════════════╝\n");
    fprintf(stderr, "\n");
    
    Tree t;
    init_tree(&t, base);
    create_base_dirs(&t);
    open_persistent_fds(&t);
    
    FILE *fp = fopen(outp, "w");
    if (!fp) die("fopen(%s) failed", outp);
    
    emit_header(fp);
    fflush(fp);
    
    foreach_cases(&t, fp, max_rows, progress);
    
    fflush(fp);
    fclose(fp);
    
    close_persistent_fds(&t);
    
    fprintf(stderr, "\n✓ DONE: wrote %s\n\n", outp);
    return 0;
}
