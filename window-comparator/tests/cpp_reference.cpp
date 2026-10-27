// Compile the unmodified C++ plugin against a small deterministic host.
// This executable is test-only; no original C++ algorithm code enters the Rust plugin.
#define _DISTINGNT_SERIALISATION_INTERNAL
#include <distingnt/api.h>
#include <distingnt/serialisation.h>
#include <algorithm>
#include <array>
#include <bit>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>
#include "reference/WindowComparator/windowComparatorAlg.cpp"
#include "reference/WindowComparator/comparatorView.cpp"
#include "reference/Common/viewBase.cpp"
#include "reference/Common/potManager.cpp"
#include "reference/Common/timeKeeper.cpp"
#include "reference/Common/helpTextHelper.cpp"

static _NT_algorithm* current;
static bool recordDraw = false;
static uint32_t drawHash = 2166136261u;
static void hashString(const std::string& s) { for (unsigned char c: s) {drawHash ^= c; drawHash *= 16777619u;} }
extern "C" const _NT_globals NT_globals = {48000,128,nullptr,0,0,0};
extern "C" int32_t NT_algorithmIndex(const _NT_algorithm*) {return 0;}
extern "C" uint32_t NT_parameterOffset() {return 0;}
static void set(uint32_t p, int16_t v) {
    v = std::max(current->parameters[p].min, std::min(current->parameters[p].max, v));
    auto ptr = const_cast<int16_t*>(current->v) + p;
    if (*ptr != v) {*ptr=v; WindowComparatorAlg::Factory.parameterChanged(current,p);}
}
extern "C" void NT_setParameterFromAudio(uint32_t, uint32_t p, int16_t v) {set(p,v);}
extern "C" void NT_setParameterFromUi(uint32_t, uint32_t p, int16_t v) {set(p,v);}
extern "C" void NT_setParameterGrayedOut(uint32_t,uint32_t,bool) {}
extern "C" void NT_updateParameterDefinition(uint32_t,uint32_t) {}
extern "C" void NT_drawShapeI(_NT_shape kind,int x0,int y0,int x1,int y1,int colour) {
    if (!recordDraw) return;
    char s[160]; std::snprintf(s,sizeof(s),"S %u %d %d %d %d %d",kind,x0,y0,x1,y1,colour); hashString(s);
}
extern "C" void NT_drawText(int x,int y,const char* str,int colour,_NT_textAlignment align,_NT_textSize size) {
    if (!recordDraw) return;
    char s[160]; std::snprintf(s,sizeof(s),"T %d %d %d %u %u %s",x,y,colour,align,size,str); hashString(s);
}
extern "C" int NT_intToString(char* b,int32_t v) {return std::sprintf(b,"%d",v);}
extern "C" int NT_floatToString(char* b,float v,int d) {return std::sprintf(b,"%.*f",d,v);}

struct Json {std::vector<std::pair<std::string,bool>> members; std::string name; size_t pos=0;};
_NT_jsonStream::_NT_jsonStream(void* p):refCon(p) {}
_NT_jsonStream::~_NT_jsonStream() {}
void _NT_jsonStream::addMemberName(const char* s) {static_cast<Json*>(refCon)->name=s;}
void _NT_jsonStream::addBoolean(bool v) {auto& j=*static_cast<Json*>(refCon);j.members.push_back({j.name,v});}
_NT_jsonParse::_NT_jsonParse(void* p,int v):refCon(p),i(v) {}
_NT_jsonParse::~_NT_jsonParse() {}
bool _NT_jsonParse::numberOfObjectMembers(int& n) {n=static_cast<Json*>(refCon)->members.size();return true;}
bool _NT_jsonParse::matchName(const char* name) {auto& j=*static_cast<Json*>(refCon);return j.members[j.pos].first==name;}
bool _NT_jsonParse::boolean(bool& v) {auto& j=*static_cast<Json*>(refCon);v=j.members[j.pos++].second;return true;}
bool _NT_jsonParse::skipMember() {static_cast<Json*>(refCon)->pos++;return true;}

static int offset(int ch) {return ch==0 ? 4 : 33+(ch-1)*19;}
static void snapshot(_NT_algorithm* a,int n,const char* phase) {
    drawHash=2166136261u;recordDraw=true;WindowComparatorAlg::Factory.draw(a);recordDraw=false;
    Json json; {_NT_jsonStream stream(&json);WindowComparatorAlg::Factory.serialise(a,stream);}
    std::printf("D %d %s %08x %d %d\n",n,phase,drawHash,json.members[0].second,json.members[1].second);
}
int main() {
    auto& f=WindowComparatorAlg::Factory;
    for(int n=1;n<=8;n++) {
        _NT_algorithmRequirements req; f.calculateRequirements(req,&n);
        auto ram=std::calloc(1,req.sram); _NT_algorithmMemoryPtrs mem{static_cast<uint8_t*>(ram),nullptr,nullptr,nullptr};
        auto a=f.construct(mem,req,&n); current=a;
        std::vector<int16_t> values(req.numParameters);
        for(unsigned p=0;p<req.numParameters;p++) {
            auto& d=const_cast<_NT_parameter&>(a->parameters[p]);
            // API v14 permits 64 buses; preserve the algorithm's original logic
            // while extending its explicitly written legacy parameter limits.
            if(d.unit>=kNT_unitAudioInput && d.unit<=kNT_unitCvOutput) d.max=kNT_lastBus;
            values[p]=d.def;
            std::printf("P %d %u %s %d %d %d %u %u\n",n,p,d.name,d.min,d.max,d.def,d.unit,d.scaling);
        }
        a->v=a->vIncludingCommon=values.data();
        for(unsigned p=0;p<a->parameterPages->numPages;p++) {
            auto& d=a->parameterPages->pages[p]; std::printf("G %d %s %u",n,d.name,d.group);
            for(int j=0;j<d.numParams;j++) std::printf(" %u",d.params[j]); std::puts("");
        }
        // Exercise recursive parameter updates and range-definition changes.
        for(auto [p,v]: std::vector<std::pair<int,int>>{{5,-2000},{7,0},{0,-10},{1,10},{8,4000}}) {
            values[p]=v;f.parameterChanged(a,p);
        }
        for(unsigned p=0;p<req.numParameters;p++) std::printf("V %d %u %d %d %d\n",n,p,values[p],a->parameters[p].min,a->parameters[p].max);
        for(int ch=0;ch<n;ch++) {
            int o=offset(ch);values[o]=ch%3==2?2:1;
            values[o+1]=-1000+ch*100;values[o+2]=1000+ch*100;
            values[o+5]=ch%2;values[o+6]=5000;values[o+7]=ch*10;
            for(int j=0;j<4;j++) values[o+11+j]=2+j;
        }
        if(n>1) {
            for(int j=0;j<12;j++) values[19+j]=6+j;
            values[31]=18;values[32]=19;
            for(int ch=1;ch<n;ch++) for(int j=0;j<4;j++) values[offset(ch)+15+j]=20+j;
        }
        float inputs[]={-10,-3,-1,0,1,3,10,0};
        for(int tick=0;tick<32;tick++) {
            const int frames=std::array{4,16,48,128}[tick%4];
            std::vector<float> buses(kNT_lastBus*frames,-123.f);std::fill_n(buses.data(),frames,inputs[tick%8]);
            f.step(a,buses.data(),frames/4);std::printf("B %d %d",n,tick);
            for(float v:buses) {auto bits=(v!=v)?0x7fc00000u:std::bit_cast<uint32_t>(v);std::printf(" %08x",bits);}std::puts("");
        }
        snapshot(a,n,"default");
        _NT_uiData ui{{0.5,0.5,0.5},0,0,{0,0},{0,0}};f.customUi(a,ui);
        ui.encoders[0]=1;ui.controls=kNT_potL;ui.pots[0]=0.505f;f.customUi(a,ui);snapshot(a,n,"pot");
        ui.encoders[0]=0;ui.controls=kNT_encoderButtonR;f.customUi(a,ui);
        ui.controls=0;ui.lastButtons=kNT_encoderButtonR;f.customUi(a,ui);snapshot(a,n,"locked");
        ui.controls=kNT_encoderButtonR;ui.lastButtons=0;f.customUi(a,ui);
        float buses[kNT_lastBus*48]={};for(int i=0;i<300;i++)f.step(a,buses,12); // 300ms
        snapshot(a,n,"long");
        ui.controls=0;ui.lastButtons=kNT_encoderButtonR;f.customUi(a,ui);
        Json j;j.members={{"Unknown",true},{"BoundsEditMode",true},{"Editable",true}};
        {_NT_jsonParse parse(&j,0);if(!f.deserialise(a,parse))return 2;}
        snapshot(a,n,"restored");
        std::free(ram);
    }
}
