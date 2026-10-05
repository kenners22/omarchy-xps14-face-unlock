// dlib, for the face side of a Howdy scan: the same models and calls dlib's
// Python bindings make (detector, landmarks, face descriptor). dlib is a C++
// template library with no C interface, so this file is the bridge; the Rust
// side does everything else, including the image preparation (imgproc.rs).

#include <cstring>
#include <exception>
#include <vector>

#include <dlib/dnn.h>
#include <dlib/image_processing.h>
#include <dlib/image_processing/frontal_face_detector.h>
#include <dlib/image_transforms.h>

using namespace dlib;

// From OpenBLAS, linked ahead of dlib so dlib's BLAS calls go to it (build.rs).
extern "C" void openblas_set_num_threads(int);

namespace {

// dlib_face_recognition_resnet_model_v1: the network dlib's Python
// face_recognition_model_v1 builds (tools/python/src/face_recognition.cpp).
template <template <int, template <typename> class, int, typename> class block, int N,
          template <typename> class BN, typename SUBNET>
using residual = add_prev1<block<N, BN, 1, tag1<SUBNET>>>;
template <template <int, template <typename> class, int, typename> class block, int N,
          template <typename> class BN, typename SUBNET>
using residual_down = add_prev2<avg_pool<2, 2, 2, 2, skip1<tag2<block<N, BN, 2, tag1<SUBNET>>>>>>;
template <int N, template <typename> class BN, int stride, typename SUBNET>
using block = BN<con<N, 3, 3, 1, 1, relu<BN<con<N, 3, 3, stride, stride, SUBNET>>>>>;
template <int N, typename SUBNET> using ares = relu<residual<block, N, affine, SUBNET>>;
template <int N, typename SUBNET> using ares_down = relu<residual_down<block, N, affine, SUBNET>>;
template <typename SUBNET> using alevel0 = ares_down<256, SUBNET>;
template <typename SUBNET> using alevel1 = ares<256, ares<256, ares_down<256, SUBNET>>>;
template <typename SUBNET> using alevel2 = ares<128, ares<128, ares_down<128, SUBNET>>>;
template <typename SUBNET> using alevel3 = ares<64, ares<64, ares<64, ares_down<64, SUBNET>>>>;
template <typename SUBNET> using alevel4 = ares<32, ares<32, ares<32, SUBNET>>>;
using anet_type = loss_metric<fc_no_bias<128, avg_pool_everything<alevel0<alevel1<alevel2<alevel3<alevel4<
    max_pool<3, 3, 2, 2, relu<affine<con<32, 7, 7, 2, 2, input_rgb_image_sized<150>>>>>>>>>>>>>;

// mmod_human_face_detector: dlib's Python cnn_face_detection_model_v1
// (tools/python/src/cnn_face_detector.cpp). Only used with use_cnn = true.
template <long num_filters, typename SUBNET> using con5d = con<num_filters, 5, 5, 2, 2, SUBNET>;
template <long num_filters, typename SUBNET> using con5 = con<num_filters, 5, 5, 1, 1, SUBNET>;
template <typename SUBNET>
using downsampler = relu<affine<con5d<32, relu<affine<con5d<32, relu<affine<con5d<16, SUBNET>>>>>>>>>;
template <typename SUBNET> using rcon5 = relu<affine<con5<45, SUBNET>>>;
using cnn_net_type =
    loss_mmod<con<1, 9, 9, 1, 1, rcon5<rcon5<rcon5<downsampler<input_rgb_image_pyramid<pyramid_down<6>>>>>>>>;

void set_err(char *err, size_t len, const char *msg) {
    if (err && len) {
        std::strncpy(err, msg, len - 1);
        err[len - 1] = 0;
    }
}

// A packed 8-bit grey buffer into a dlib image.
template <typename image_type>
void from_gray(const unsigned char *src, int w, int h, image_type &dst) {
    dst.set_size(h, w);
    image_view<image_type> v(dst);
    for (int r = 0; r < h; r++)
        for (int c = 0; c < w; c++) assign_pixel(v[r][c], src[r * w + c]);
}

}  // namespace

struct fw_models {
    bool use_cnn = false;
    frontal_face_detector hog;
    cnn_net_type cnn;
    shape_predictor sp;
    anet_type net;
};

extern "C" {

fw_models *fw_load(const char *sp_path, const char *rec_path, const char *cnn_path, char *err, size_t errlen) {
    try {
        // One thread: as fast as many for one 150px face, and no idle pool
        // spinning on every core after each scan.
        openblas_set_num_threads(1);
        auto *m = new fw_models;
        if (cnn_path) {
            m->use_cnn = true;
            deserialize(cnn_path) >> m->cnn;
        } else {
            m->hog = get_frontal_face_detector();
        }
        deserialize(sp_path) >> m->sp;
        deserialize(rec_path) >> m->net;
        return m;
    } catch (std::exception &e) {
        set_err(err, errlen, e.what());
    } catch (...) {
        set_err(err, errlen, "unknown error loading models");
    }
    return nullptr;
}

void fw_free(fw_models *m) { delete m; }

// Same as dlib's Python detector(img, upsample_num_times): upsample with
// pyramid_up, detect, map the boxes back down.
static std::vector<rectangle> detect(fw_models *m, const unsigned char *gs, int w, int h, int upsample) {
    pyramid_down<2> pyr;
    std::vector<rectangle> out;
    if (m->use_cnn) {
        matrix<rgb_pixel> img;
        from_gray(gs, w, h, img);
        for (int i = 0; i < upsample; i++) pyramid_up(img, pyr);
        for (auto &d : m->cnn(img)) out.push_back(rectangle(pyr.rect_down(d.rect, upsample)));
        return out;
    }
    array2d<unsigned char> img;
    from_gray(gs, w, h, img);
    for (int i = 0; i < upsample; i++) pyramid_up(img, pyr);
    for (auto &r : m->hog(img)) out.push_back(upsample ? rectangle(pyr.rect_down(r, upsample)) : r);
    return out;
}

// The faces in one prepared frame, as Howdy's loop finds them:
//   for fl in face_detector(gsframe, 1):
//       compute_face_descriptor(frame, pose_predictor(frame, fl), 1)
//   gs      the CLAHE'd frame, resized and rotated (detection runs on this)
//   frame   the plain frame, resized and rotated the same way; Howdy hands
//           dlib a BGR copy, i.e. this grey image three times over
//   desc    out: one 128-float descriptor per face, up to max_faces
// Returns the number of faces, or -1 on error (message in err).
int fw_faces(fw_models *m, const unsigned char *gs, const unsigned char *frame, int w, int h, float *desc,
             int max_faces, char *err, size_t errlen) {
    try {
        auto faces = detect(m, gs, w, h, 1);
        if (faces.empty()) return 0;
        matrix<rgb_pixel> rgb;
        from_gray(frame, w, h, rgb);
        int n = 0;
        for (auto &r : faces) {
            if (n == max_faces) break;
            auto shape = m->sp(rgb, r);
            // num_jitters = 1 is the no-jitter path: one 150px chip, padding 0.25
            matrix<rgb_pixel> chip;
            extract_image_chip(rgb, get_face_chip_details(shape, 150, 0.25), chip);
            matrix<float, 0, 1> d = m->net(chip);
            std::memcpy(desc + 128 * n, &d(0), 128 * sizeof(float));
            n++;
        }
        return n;
    } catch (std::exception &e) {
        set_err(err, errlen, e.what());
    } catch (...) {
        set_err(err, errlen, "unknown error");
    }
    return -1;
}

}  // extern "C"
