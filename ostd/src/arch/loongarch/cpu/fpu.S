/* SPDX-License-Identifier: MPL-2.0 */

.equ LOONGARCH_CSR_EUEN, {LOONGARCH_CSR_EUEN}
.equ EUEN_SXE, {EUEN_SXE}
.equ EUEN_ASXE, {EUEN_ASXE}
.equ REGISTER_STRIDE, {REGISTER_STRIDE}
.equ FCC_OFFSET, {FCC_OFFSET}
.equ FCSR_OFFSET, {FCSR_OFFSET}

.text

.global save_fpu_context
.balign 4
.type save_fpu_context, @function
save_fpu_context:
    csrrd   $t0, LOONGARCH_CSR_EUEN
    or      $t1, $t0, $a1
    csrwr   $t1, LOONGARCH_CSR_EUEN

    andi    $t1, $a1, EUEN_ASXE
    bnez    $t1, .Lsave_lasx
    andi    $t1, $a1, EUEN_SXE
    bnez    $t1, .Lsave_lsx
    fst.d   $f0, $a0, 0 * REGISTER_STRIDE
    fst.d   $f1, $a0, 1 * REGISTER_STRIDE
    fst.d   $f2, $a0, 2 * REGISTER_STRIDE
    fst.d   $f3, $a0, 3 * REGISTER_STRIDE
    fst.d   $f4, $a0, 4 * REGISTER_STRIDE
    fst.d   $f5, $a0, 5 * REGISTER_STRIDE
    fst.d   $f6, $a0, 6 * REGISTER_STRIDE
    fst.d   $f7, $a0, 7 * REGISTER_STRIDE
    fst.d   $f8, $a0, 8 * REGISTER_STRIDE
    fst.d   $f9, $a0, 9 * REGISTER_STRIDE
    fst.d   $f10, $a0, 10 * REGISTER_STRIDE
    fst.d   $f11, $a0, 11 * REGISTER_STRIDE
    fst.d   $f12, $a0, 12 * REGISTER_STRIDE
    fst.d   $f13, $a0, 13 * REGISTER_STRIDE
    fst.d   $f14, $a0, 14 * REGISTER_STRIDE
    fst.d   $f15, $a0, 15 * REGISTER_STRIDE
    fst.d   $f16, $a0, 16 * REGISTER_STRIDE
    fst.d   $f17, $a0, 17 * REGISTER_STRIDE
    fst.d   $f18, $a0, 18 * REGISTER_STRIDE
    fst.d   $f19, $a0, 19 * REGISTER_STRIDE
    fst.d   $f20, $a0, 20 * REGISTER_STRIDE
    fst.d   $f21, $a0, 21 * REGISTER_STRIDE
    fst.d   $f22, $a0, 22 * REGISTER_STRIDE
    fst.d   $f23, $a0, 23 * REGISTER_STRIDE
    fst.d   $f24, $a0, 24 * REGISTER_STRIDE
    fst.d   $f25, $a0, 25 * REGISTER_STRIDE
    fst.d   $f26, $a0, 26 * REGISTER_STRIDE
    fst.d   $f27, $a0, 27 * REGISTER_STRIDE
    fst.d   $f28, $a0, 28 * REGISTER_STRIDE
    fst.d   $f29, $a0, 29 * REGISTER_STRIDE
    fst.d   $f30, $a0, 30 * REGISTER_STRIDE
    fst.d   $f31, $a0, 31 * REGISTER_STRIDE
    b       .Lsave_control
.Lsave_lsx:
    vst     $vr0, $a0, 0 * REGISTER_STRIDE
    vst     $vr1, $a0, 1 * REGISTER_STRIDE
    vst     $vr2, $a0, 2 * REGISTER_STRIDE
    vst     $vr3, $a0, 3 * REGISTER_STRIDE
    vst     $vr4, $a0, 4 * REGISTER_STRIDE
    vst     $vr5, $a0, 5 * REGISTER_STRIDE
    vst     $vr6, $a0, 6 * REGISTER_STRIDE
    vst     $vr7, $a0, 7 * REGISTER_STRIDE
    vst     $vr8, $a0, 8 * REGISTER_STRIDE
    vst     $vr9, $a0, 9 * REGISTER_STRIDE
    vst     $vr10, $a0, 10 * REGISTER_STRIDE
    vst     $vr11, $a0, 11 * REGISTER_STRIDE
    vst     $vr12, $a0, 12 * REGISTER_STRIDE
    vst     $vr13, $a0, 13 * REGISTER_STRIDE
    vst     $vr14, $a0, 14 * REGISTER_STRIDE
    vst     $vr15, $a0, 15 * REGISTER_STRIDE
    vst     $vr16, $a0, 16 * REGISTER_STRIDE
    vst     $vr17, $a0, 17 * REGISTER_STRIDE
    vst     $vr18, $a0, 18 * REGISTER_STRIDE
    vst     $vr19, $a0, 19 * REGISTER_STRIDE
    vst     $vr20, $a0, 20 * REGISTER_STRIDE
    vst     $vr21, $a0, 21 * REGISTER_STRIDE
    vst     $vr22, $a0, 22 * REGISTER_STRIDE
    vst     $vr23, $a0, 23 * REGISTER_STRIDE
    vst     $vr24, $a0, 24 * REGISTER_STRIDE
    vst     $vr25, $a0, 25 * REGISTER_STRIDE
    vst     $vr26, $a0, 26 * REGISTER_STRIDE
    vst     $vr27, $a0, 27 * REGISTER_STRIDE
    vst     $vr28, $a0, 28 * REGISTER_STRIDE
    vst     $vr29, $a0, 29 * REGISTER_STRIDE
    vst     $vr30, $a0, 30 * REGISTER_STRIDE
    vst     $vr31, $a0, 31 * REGISTER_STRIDE
    b       .Lsave_control
.Lsave_lasx:
    xvst    $xr0, $a0, 0 * REGISTER_STRIDE
    xvst    $xr1, $a0, 1 * REGISTER_STRIDE
    xvst    $xr2, $a0, 2 * REGISTER_STRIDE
    xvst    $xr3, $a0, 3 * REGISTER_STRIDE
    xvst    $xr4, $a0, 4 * REGISTER_STRIDE
    xvst    $xr5, $a0, 5 * REGISTER_STRIDE
    xvst    $xr6, $a0, 6 * REGISTER_STRIDE
    xvst    $xr7, $a0, 7 * REGISTER_STRIDE
    xvst    $xr8, $a0, 8 * REGISTER_STRIDE
    xvst    $xr9, $a0, 9 * REGISTER_STRIDE
    xvst    $xr10, $a0, 10 * REGISTER_STRIDE
    xvst    $xr11, $a0, 11 * REGISTER_STRIDE
    xvst    $xr12, $a0, 12 * REGISTER_STRIDE
    xvst    $xr13, $a0, 13 * REGISTER_STRIDE
    xvst    $xr14, $a0, 14 * REGISTER_STRIDE
    xvst    $xr15, $a0, 15 * REGISTER_STRIDE
    xvst    $xr16, $a0, 16 * REGISTER_STRIDE
    xvst    $xr17, $a0, 17 * REGISTER_STRIDE
    xvst    $xr18, $a0, 18 * REGISTER_STRIDE
    xvst    $xr19, $a0, 19 * REGISTER_STRIDE
    xvst    $xr20, $a0, 20 * REGISTER_STRIDE
    xvst    $xr21, $a0, 21 * REGISTER_STRIDE
    xvst    $xr22, $a0, 22 * REGISTER_STRIDE
    xvst    $xr23, $a0, 23 * REGISTER_STRIDE
    xvst    $xr24, $a0, 24 * REGISTER_STRIDE
    xvst    $xr25, $a0, 25 * REGISTER_STRIDE
    xvst    $xr26, $a0, 26 * REGISTER_STRIDE
    xvst    $xr27, $a0, 27 * REGISTER_STRIDE
    xvst    $xr28, $a0, 28 * REGISTER_STRIDE
    xvst    $xr29, $a0, 29 * REGISTER_STRIDE
    xvst    $xr30, $a0, 30 * REGISTER_STRIDE
    xvst    $xr31, $a0, 31 * REGISTER_STRIDE

.Lsave_control:
    movcf2gr $t1, $fcc0
    st.b    $t1, $a0, FCC_OFFSET + 0
    movcf2gr $t1, $fcc1
    st.b    $t1, $a0, FCC_OFFSET + 1
    movcf2gr $t1, $fcc2
    st.b    $t1, $a0, FCC_OFFSET + 2
    movcf2gr $t1, $fcc3
    st.b    $t1, $a0, FCC_OFFSET + 3
    movcf2gr $t1, $fcc4
    st.b    $t1, $a0, FCC_OFFSET + 4
    movcf2gr $t1, $fcc5
    st.b    $t1, $a0, FCC_OFFSET + 5
    movcf2gr $t1, $fcc6
    st.b    $t1, $a0, FCC_OFFSET + 6
    movcf2gr $t1, $fcc7
    st.b    $t1, $a0, FCC_OFFSET + 7
    movfcsr2gr $t1, $fcsr0
    st.w    $t1, $a0, FCSR_OFFSET
    csrwr   $t0, LOONGARCH_CSR_EUEN
    ret
.size save_fpu_context, .-save_fpu_context

.global load_fpu_context
.balign 4
.type load_fpu_context, @function
load_fpu_context:
    csrrd   $t0, LOONGARCH_CSR_EUEN
    or      $t1, $t0, $a1
    csrwr   $t1, LOONGARCH_CSR_EUEN

    andi    $t1, $a1, EUEN_ASXE
    bnez    $t1, .Lload_lasx
    andi    $t1, $a1, EUEN_SXE
    bnez    $t1, .Lload_lsx
    fld.d   $f0, $a0, 0 * REGISTER_STRIDE
    fld.d   $f1, $a0, 1 * REGISTER_STRIDE
    fld.d   $f2, $a0, 2 * REGISTER_STRIDE
    fld.d   $f3, $a0, 3 * REGISTER_STRIDE
    fld.d   $f4, $a0, 4 * REGISTER_STRIDE
    fld.d   $f5, $a0, 5 * REGISTER_STRIDE
    fld.d   $f6, $a0, 6 * REGISTER_STRIDE
    fld.d   $f7, $a0, 7 * REGISTER_STRIDE
    fld.d   $f8, $a0, 8 * REGISTER_STRIDE
    fld.d   $f9, $a0, 9 * REGISTER_STRIDE
    fld.d   $f10, $a0, 10 * REGISTER_STRIDE
    fld.d   $f11, $a0, 11 * REGISTER_STRIDE
    fld.d   $f12, $a0, 12 * REGISTER_STRIDE
    fld.d   $f13, $a0, 13 * REGISTER_STRIDE
    fld.d   $f14, $a0, 14 * REGISTER_STRIDE
    fld.d   $f15, $a0, 15 * REGISTER_STRIDE
    fld.d   $f16, $a0, 16 * REGISTER_STRIDE
    fld.d   $f17, $a0, 17 * REGISTER_STRIDE
    fld.d   $f18, $a0, 18 * REGISTER_STRIDE
    fld.d   $f19, $a0, 19 * REGISTER_STRIDE
    fld.d   $f20, $a0, 20 * REGISTER_STRIDE
    fld.d   $f21, $a0, 21 * REGISTER_STRIDE
    fld.d   $f22, $a0, 22 * REGISTER_STRIDE
    fld.d   $f23, $a0, 23 * REGISTER_STRIDE
    fld.d   $f24, $a0, 24 * REGISTER_STRIDE
    fld.d   $f25, $a0, 25 * REGISTER_STRIDE
    fld.d   $f26, $a0, 26 * REGISTER_STRIDE
    fld.d   $f27, $a0, 27 * REGISTER_STRIDE
    fld.d   $f28, $a0, 28 * REGISTER_STRIDE
    fld.d   $f29, $a0, 29 * REGISTER_STRIDE
    fld.d   $f30, $a0, 30 * REGISTER_STRIDE
    fld.d   $f31, $a0, 31 * REGISTER_STRIDE
    b       .Lload_control
.Lload_lsx:
    vld     $vr0, $a0, 0 * REGISTER_STRIDE
    vld     $vr1, $a0, 1 * REGISTER_STRIDE
    vld     $vr2, $a0, 2 * REGISTER_STRIDE
    vld     $vr3, $a0, 3 * REGISTER_STRIDE
    vld     $vr4, $a0, 4 * REGISTER_STRIDE
    vld     $vr5, $a0, 5 * REGISTER_STRIDE
    vld     $vr6, $a0, 6 * REGISTER_STRIDE
    vld     $vr7, $a0, 7 * REGISTER_STRIDE
    vld     $vr8, $a0, 8 * REGISTER_STRIDE
    vld     $vr9, $a0, 9 * REGISTER_STRIDE
    vld     $vr10, $a0, 10 * REGISTER_STRIDE
    vld     $vr11, $a0, 11 * REGISTER_STRIDE
    vld     $vr12, $a0, 12 * REGISTER_STRIDE
    vld     $vr13, $a0, 13 * REGISTER_STRIDE
    vld     $vr14, $a0, 14 * REGISTER_STRIDE
    vld     $vr15, $a0, 15 * REGISTER_STRIDE
    vld     $vr16, $a0, 16 * REGISTER_STRIDE
    vld     $vr17, $a0, 17 * REGISTER_STRIDE
    vld     $vr18, $a0, 18 * REGISTER_STRIDE
    vld     $vr19, $a0, 19 * REGISTER_STRIDE
    vld     $vr20, $a0, 20 * REGISTER_STRIDE
    vld     $vr21, $a0, 21 * REGISTER_STRIDE
    vld     $vr22, $a0, 22 * REGISTER_STRIDE
    vld     $vr23, $a0, 23 * REGISTER_STRIDE
    vld     $vr24, $a0, 24 * REGISTER_STRIDE
    vld     $vr25, $a0, 25 * REGISTER_STRIDE
    vld     $vr26, $a0, 26 * REGISTER_STRIDE
    vld     $vr27, $a0, 27 * REGISTER_STRIDE
    vld     $vr28, $a0, 28 * REGISTER_STRIDE
    vld     $vr29, $a0, 29 * REGISTER_STRIDE
    vld     $vr30, $a0, 30 * REGISTER_STRIDE
    vld     $vr31, $a0, 31 * REGISTER_STRIDE
    b       .Lload_control
.Lload_lasx:
    xvld    $xr0, $a0, 0 * REGISTER_STRIDE
    xvld    $xr1, $a0, 1 * REGISTER_STRIDE
    xvld    $xr2, $a0, 2 * REGISTER_STRIDE
    xvld    $xr3, $a0, 3 * REGISTER_STRIDE
    xvld    $xr4, $a0, 4 * REGISTER_STRIDE
    xvld    $xr5, $a0, 5 * REGISTER_STRIDE
    xvld    $xr6, $a0, 6 * REGISTER_STRIDE
    xvld    $xr7, $a0, 7 * REGISTER_STRIDE
    xvld    $xr8, $a0, 8 * REGISTER_STRIDE
    xvld    $xr9, $a0, 9 * REGISTER_STRIDE
    xvld    $xr10, $a0, 10 * REGISTER_STRIDE
    xvld    $xr11, $a0, 11 * REGISTER_STRIDE
    xvld    $xr12, $a0, 12 * REGISTER_STRIDE
    xvld    $xr13, $a0, 13 * REGISTER_STRIDE
    xvld    $xr14, $a0, 14 * REGISTER_STRIDE
    xvld    $xr15, $a0, 15 * REGISTER_STRIDE
    xvld    $xr16, $a0, 16 * REGISTER_STRIDE
    xvld    $xr17, $a0, 17 * REGISTER_STRIDE
    xvld    $xr18, $a0, 18 * REGISTER_STRIDE
    xvld    $xr19, $a0, 19 * REGISTER_STRIDE
    xvld    $xr20, $a0, 20 * REGISTER_STRIDE
    xvld    $xr21, $a0, 21 * REGISTER_STRIDE
    xvld    $xr22, $a0, 22 * REGISTER_STRIDE
    xvld    $xr23, $a0, 23 * REGISTER_STRIDE
    xvld    $xr24, $a0, 24 * REGISTER_STRIDE
    xvld    $xr25, $a0, 25 * REGISTER_STRIDE
    xvld    $xr26, $a0, 26 * REGISTER_STRIDE
    xvld    $xr27, $a0, 27 * REGISTER_STRIDE
    xvld    $xr28, $a0, 28 * REGISTER_STRIDE
    xvld    $xr29, $a0, 29 * REGISTER_STRIDE
    xvld    $xr30, $a0, 30 * REGISTER_STRIDE
    xvld    $xr31, $a0, 31 * REGISTER_STRIDE

.Lload_control:
    ld.bu   $t1, $a0, FCC_OFFSET + 0
    movgr2cf $fcc0, $t1
    ld.bu   $t1, $a0, FCC_OFFSET + 1
    movgr2cf $fcc1, $t1
    ld.bu   $t1, $a0, FCC_OFFSET + 2
    movgr2cf $fcc2, $t1
    ld.bu   $t1, $a0, FCC_OFFSET + 3
    movgr2cf $fcc3, $t1
    ld.bu   $t1, $a0, FCC_OFFSET + 4
    movgr2cf $fcc4, $t1
    ld.bu   $t1, $a0, FCC_OFFSET + 5
    movgr2cf $fcc5, $t1
    ld.bu   $t1, $a0, FCC_OFFSET + 6
    movgr2cf $fcc6, $t1
    ld.bu   $t1, $a0, FCC_OFFSET + 7
    movgr2cf $fcc7, $t1
    ld.wu   $t1, $a0, FCSR_OFFSET
    movgr2fcsr $fcsr0, $t1
    csrwr   $t0, LOONGARCH_CSR_EUEN
    ret
.size load_fpu_context, .-load_fpu_context
