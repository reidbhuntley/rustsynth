use std::{any::Any, collections::HashMap};

use arr_macro::arr;
use rodio::Source;
use seahash::SeaHasher;

use crate::{constants::*, midi::MidiEvents, output::AudioOutput};

use self::private::{
    BufferInPort, BufferOutPort, ModuleBuffersDescriptor, ModuleBuffersInInternal,
    ModuleBuffersOutInternal,
};

#[derive(Clone, Default)]
struct BuildHasher;

impl std::hash::BuildHasher for BuildHasher {
    type Hasher = SeaHasher;

    #[inline(always)]
    fn build_hasher(&self) -> Self::Hasher {
        SeaHasher::new()
    }
}

pub trait BufferElem: 'static + private::BufferElemSealed + Default + Clone {
    fn new_buffer(self) -> Buffer<Self> {
        arr![self.clone(); 512]
    }

    fn new_vec<T: BufferElem>(len: usize) -> Vec<T> {
        std::iter::repeat(T::default()).take(len).collect()
    }
}

impl BufferElem for f32 {}

impl BufferElem for MidiEvents {}

mod private {
    use std::sync::atomic::AtomicUsize;

    use super::{
        BufferElem, BufferInPorts, BufferOutPorts, BuffersInExt, BuffersOutExt,
        ModuleBuffersDescriptorsAll, ModuleBuffersIn, ModuleBuffersOut,
    };

    pub struct BufferOutPort<T: BufferElem> {
        pub buffer: super::Buffer<T>,
        pub dependents: Vec<super::ModuleBufferInHandle<T>>,
    }

    pub enum BufferInPort<T: BufferElem> {
        OutBuffer(super::ModuleBufferOutHandle<T>),
        Constant(super::Buffer<T>),
    }

    impl<T: BufferElem> BufferInPort<T> {
        pub fn with_constant(value: T) -> Self {
            Self::Constant(T::new_buffer(value))
        }
    }

    impl<T: BufferElem> Default for BufferInPort<T> {
        fn default() -> Self {
            Self::with_constant(T::default())
        }
    }

    #[derive(Default)]
    pub struct ModuleBuffersDescriptor<T: BufferElem> {
        pub buf_in: Vec<T>,
        pub buf_out: Vec<()>,
    }

    #[derive(Default)]
    pub struct ModuleBuffersInInternal {
        pub num_dependencies: usize,
        pub num_finished_dependencies: AtomicUsize,
        buf_signal: BufferInPorts<f32>,
        buf_midi: BufferInPorts<crate::midi::MidiEvents>,
    }

    impl ModuleBuffersInInternal {
        pub fn new(descriptors: &ModuleBuffersDescriptorsAll) -> Self {
            let mut out = Self::default();
            out.add_num_buffers_all(&descriptors);
            out
        }

        pub fn add_num_buffers_all(&mut self, descriptors: &ModuleBuffersDescriptorsAll) {
            Self::add_num_buffers(&mut self.buf_signal, &descriptors.buf_signal);
            Self::add_num_buffers(&mut self.buf_midi, &descriptors.buf_midi);
        }

        pub fn add_num_buffers<T: BufferElem>(
            buffers: &mut BufferInPorts<T>,
            defaults: &ModuleBuffersDescriptor<T>,
        ) {
            for default in defaults.buf_in.iter() {
                buffers.push(BufferInPort::with_constant(default.clone()));
            }
        }
    }

    #[derive(Default)]
    pub struct ModuleBuffersOutInternal {
        buf_signal: BufferOutPorts<f32>,
        buf_midi: BufferOutPorts<crate::midi::MidiEvents>,
    }

    impl ModuleBuffersOutInternal {
        pub fn new(descriptors: &ModuleBuffersDescriptorsAll) -> Self {
            let mut out = Self::default();
            out.add_num_buffers_all(&descriptors);
            out
        }

        pub fn add_num_buffers_all(&mut self, descriptors: &ModuleBuffersDescriptorsAll) {
            Self::add_num_buffers(&mut self.buf_signal, &descriptors.buf_signal);
            Self::add_num_buffers(&mut self.buf_midi, &descriptors.buf_midi);
        }

        pub fn add_num_buffers<T: BufferElem>(
            buffers: &mut BufferOutPorts<T>,
            descriptor: &ModuleBuffersDescriptor<T>,
        ) {
            for _ in descriptor.buf_out.iter() {
                buffers.push(BufferOutPort {
                    buffer: T::new_buffer(T::default()),
                    dependents: Vec::new(),
                });
            }
        }
    }

    pub trait BufferElemSealed {
        fn get_buffers_in(buf_in: &ModuleBuffersInInternal) -> &BufferInPorts<Self>
        where
            Self: Sized + BufferElem;

        fn get_buffers_in_mut(buf_in: &mut ModuleBuffersInInternal) -> &mut BufferInPorts<Self>
        where
            Self: Sized + BufferElem;

        fn get_ext_buffers_in(buf_in: &ModuleBuffersIn) -> &BuffersInExt<Self>
        where
            Self: Sized + BufferElem;

        fn get_buffers_out(buf_out: &ModuleBuffersOutInternal) -> &BufferOutPorts<Self>
        where
            Self: Sized + BufferElem;

        fn get_buffers_out_mut(buf_out: &mut ModuleBuffersOutInternal) -> &mut BufferOutPorts<Self>
        where
            Self: Sized + BufferElem;

        fn get_ext_buffers_out(buf_out: &ModuleBuffersOut) -> &BuffersOutExt<Self>
        where
            Self: Sized + BufferElem;

        fn get_descriptor(
            descriptors: &mut ModuleBuffersDescriptorsAll,
        ) -> &mut ModuleBuffersDescriptor<Self>
        where
            Self: Sized + BufferElem;
    }
    impl BufferElemSealed for f32 {
        fn get_buffers_in(buf_in: &ModuleBuffersInInternal) -> &BufferInPorts<Self> {
            &buf_in.buf_signal
        }

        fn get_buffers_in_mut(buf_in: &mut ModuleBuffersInInternal) -> &mut BufferInPorts<Self> {
            &mut buf_in.buf_signal
        }

        fn get_ext_buffers_in(buf_in: &ModuleBuffersIn) -> &BuffersInExt<Self> {
            &buf_in.buf_signal
        }

        fn get_buffers_out(buf_out: &ModuleBuffersOutInternal) -> &BufferOutPorts<Self> {
            &buf_out.buf_signal
        }

        fn get_buffers_out_mut(
            buf_out: &mut ModuleBuffersOutInternal,
        ) -> &mut BufferOutPorts<Self> {
            &mut buf_out.buf_signal
        }

        fn get_ext_buffers_out(buf_out: &ModuleBuffersOut) -> &BuffersOutExt<Self> {
            &buf_out.buf_signal
        }

        fn get_descriptor(
            descriptors: &mut ModuleBuffersDescriptorsAll,
        ) -> &mut ModuleBuffersDescriptor<Self> {
            &mut descriptors.buf_signal
        }
    }
    impl BufferElemSealed for crate::midi::MidiEvents {
        fn get_buffers_in(buf_in: &ModuleBuffersInInternal) -> &BufferInPorts<Self> {
            &buf_in.buf_midi
        }

        fn get_buffers_in_mut(buf_in: &mut ModuleBuffersInInternal) -> &mut BufferInPorts<Self> {
            &mut buf_in.buf_midi
        }

        fn get_ext_buffers_in(buf_in: &ModuleBuffersIn) -> &BuffersInExt<Self> {
            &buf_in.buf_midi
        }

        fn get_buffers_out(buf_out: &ModuleBuffersOutInternal) -> &BufferOutPorts<Self> {
            &buf_out.buf_midi
        }

        fn get_buffers_out_mut(
            buf_out: &mut ModuleBuffersOutInternal,
        ) -> &mut BufferOutPorts<Self> {
            &mut buf_out.buf_midi
        }

        fn get_ext_buffers_out(buf_out: &ModuleBuffersOut) -> &BuffersOutExt<Self> {
            &buf_out.buf_midi
        }

        fn get_descriptor(
            descriptors: &mut ModuleBuffersDescriptorsAll,
        ) -> &mut ModuleBuffersDescriptor<Self> {
            &mut descriptors.buf_midi
        }
    }
}

pub type Buffer<T> = [T; BUFFER_LEN];

type BufferInPorts<T> = Vec<BufferInPort<T>>;

type BufferOutPorts<T> = Vec<BufferOutPort<T>>;

#[derive(Default, Clone, Eq)]
pub struct BufferInHandle<T: BufferElem> {
    _marker: std::marker::PhantomData<T>,
    idx: usize,
}

impl<T: BufferElem> PartialEq for BufferInHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.idx == other.idx
    }
}

#[derive(Default, Clone, Eq)]
pub struct BufferOutHandle<T: BufferElem> {
    _marker: std::marker::PhantomData<T>,
    idx: usize,
}

impl<T: BufferElem> PartialEq for BufferOutHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.idx == other.idx
    }
}

#[derive(Clone, Eq)]
pub struct ModuleBufferInHandle<T: BufferElem> {
    module_handle: ModuleHandle,
    buf_handle: BufferInHandle<T>,
}

impl<T: BufferElem> PartialEq for ModuleBufferInHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.module_handle == other.module_handle && self.buf_handle == other.buf_handle
    }
}

#[derive(Clone, Eq)]
pub struct ModuleBufferOutHandle<T: BufferElem> {
    module_handle: ModuleHandle,
    buf_handle: BufferOutHandle<T>,
}

impl<T: BufferElem> PartialEq for ModuleBufferOutHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.module_handle == other.module_handle && self.buf_handle == other.buf_handle
    }
}

#[derive(Default)]
pub struct ModuleBuffersDescriptorsAll {
    buf_signal: ModuleBuffersDescriptor<f32>,
    buf_midi: ModuleBuffersDescriptor<MidiEvents>,
}

pub struct ModuleDescriptor<T> {
    initial_data: Box<T>,
    buffers_descriptors: ModuleBuffersDescriptorsAll,
}

impl<T> ModuleDescriptor<T> {
    pub fn new(initial_data: T) -> Self {
        Self {
            initial_data: Box::new(initial_data),
            buffers_descriptors: Default::default(),
        }
    }

    pub fn with_buf_in_default<E: BufferElem>(mut self, default: E) -> Self {
        E::get_descriptor(&mut self.buffers_descriptors)
            .buf_in
            .push(default);
        self
    }

    pub fn with_buf_in<E: BufferElem>(self) -> Self {
        self.with_buf_in_default::<E>(Default::default())
    }

    pub fn with_buf_out<E: BufferElem>(mut self) -> Self {
        E::get_descriptor(&mut self.buffers_descriptors)
            .buf_out
            .push(());
        self
    }
}

type BuffersInExt<T> = Vec<*const Buffer<T>>;

pub struct ModuleBuffersIn {
    buf_signal: BuffersInExt<f32>,
    buf_midi: BuffersInExt<MidiEvents>,
}

impl ModuleBuffersIn {
    pub fn get<T: BufferElem>(&self, idx: usize) -> &Buffer<T> {
        let bufs = T::get_ext_buffers_in(&self)[idx];
        unsafe { &*bufs }
    }

    pub fn iter<T: BufferElem>(&self) -> impl Iterator<Item = &Buffer<T>> + '_ {
        T::get_ext_buffers_in(&self)
            .iter()
            .map(|&bufs| unsafe { &*bufs })
    }
}

type BuffersOutExt<T> = Vec<*mut Buffer<T>>;

pub struct ModuleBuffersOut {
    buf_signal: Vec<*mut Buffer<f32>>,
    buf_midi: Vec<*mut Buffer<MidiEvents>>,
}

impl ModuleBuffersOut {
    pub fn get<T: BufferElem>(&mut self, idx: usize) -> &mut Buffer<T> {
        let bufs = T::get_ext_buffers_out(&self)[idx];
        unsafe { &mut *bufs }
    }

    pub fn iter_mut<T: BufferElem>(&mut self) -> impl Iterator<Item = &mut Buffer<T>> + '_ {
        T::get_ext_buffers_out(self)
            .iter()
            .map(|&bufs| unsafe { &mut *bufs })
    }
}

pub trait ModuleTypes {
    type Settings;
}

pub trait Module: 'static + Any {
    fn init(settings: Self::Settings) -> ModuleDescriptor<Self>
    where
        Self: Sized + ModuleTypes;
    fn fill_buffers(&mut self, buffers_in: &ModuleBuffersIn, buffers_out: &mut ModuleBuffersOut);
}

struct ModuleInternals {
    module: Box<dyn Module>,
    buf_in: ModuleBuffersInInternal,
    buf_out: ModuleBuffersOutInternal,
}

impl ModuleInternals {
    fn new<T: Module + ModuleTypes>(settings: T::Settings) -> Self {
        let descriptor = T::init(settings);
        Self {
            module: descriptor.initial_data,
            buf_in: ModuleBuffersInInternal::new(&descriptor.buffers_descriptors),
            buf_out: ModuleBuffersOutInternal::new(&descriptor.buffers_descriptors),
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct ModuleHandle {
    idx: usize,
}

impl ModuleHandle {
    pub fn buf_in<T: BufferElem>(&self, port_idx: usize) -> ModuleBufferInHandle<T> {
        ModuleBufferInHandle {
            module_handle: self.clone(),
            buf_handle: BufferInHandle {
                idx: port_idx,
                ..Default::default()
            },
        }
    }

    pub fn buf_out<T: BufferElem>(&self, buffer_idx: usize) -> ModuleBufferOutHandle<T> {
        ModuleBufferOutHandle {
            module_handle: self.clone(),
            buf_handle: BufferOutHandle {
                idx: buffer_idx,
                ..Default::default()
            },
        }
    }
}

pub struct Host {
    modules: HashMap<usize, ModuleInternals, BuildHasher>,
    next_idx: usize,
    output: rodio::source::Stoppable<AudioOutput>,
    output_handle: ModuleHandle,
}

impl Host {
    pub fn new() -> Self {
        let output = AudioOutput::new();
        let mut out = Self {
            modules: HashMap::default(),
            next_idx: 0,
            output: output.clone().stoppable(),
            output_handle: Default::default(),
        };
        out.output_handle = out.create_module::<AudioOutput>(output);
        out
    }

    pub fn output_module(&self) -> ModuleHandle {
        self.output_handle.clone()
    }

    pub fn create_module<T: Module + ModuleTypes>(
        &mut self,
        settings: T::Settings,
    ) -> ModuleHandle {
        let module = ModuleInternals::new::<T>(settings);
        let idx = self.next_idx;
        self.next_idx += 1;
        self.modules.insert(idx, module);
        ModuleHandle { idx }
    }

    fn set_buffer_in<T: BufferElem>(
        &mut self,
        port_handle: ModuleBufferInHandle<T>,
        new: BufferInPort<T>,
    ) {
        let module_in = self
            .modules
            .get_mut(&port_handle.module_handle.idx)
            .unwrap();

        let port = &T::get_buffers_in(&module_in.buf_in)[port_handle.buf_handle.idx];
        let (old_out, new_out) = match port {
            BufferInPort::OutBuffer(out) => (
                Some(out.clone()),
                match &new {
                    BufferInPort::Constant(_) => {
                        module_in.buf_in.num_dependencies -= 1;
                        None
                    }
                    BufferInPort::OutBuffer(new_out) => Some(new_out.clone()),
                },
            ),
            BufferInPort::Constant(_) => (
                None,
                match &new {
                    BufferInPort::Constant(_) => None,
                    BufferInPort::OutBuffer(new_out) => {
                        module_in.buf_in.num_dependencies += 1;
                        Some(new_out.clone())
                    }
                },
            ),
        };

        T::get_buffers_in_mut(&mut module_in.buf_in)[port_handle.buf_handle.idx] = new;

        if let Some(old_out) = old_out {
            let module_out = self.modules.get_mut(&old_out.module_handle.idx).unwrap();
            T::get_buffers_out_mut(&mut module_out.buf_out)[old_out.buf_handle.idx]
                .dependents
                .retain(|d| d != &port_handle);
        }

        if let Some(new_out) = new_out {
            let module_out = self.modules.get_mut(&new_out.module_handle.idx).unwrap();
            T::get_buffers_out_mut(&mut module_out.buf_out)[new_out.buf_handle.idx]
                .dependents
                .push(port_handle);
        }
    }

    pub fn link<T: BufferElem>(
        &mut self,
        handle_out: ModuleBufferOutHandle<T>,
        handle_in: ModuleBufferInHandle<T>,
    ) {
        self.set_buffer_in(handle_in, BufferInPort::OutBuffer(handle_out));
    }

    pub fn link_value<T: BufferElem>(&mut self, value: T, handle_in: ModuleBufferInHandle<T>) {
        self.set_buffer_in(handle_in, BufferInPort::with_constant(value));
    }

    pub fn destroy_module(&mut self, handle: &ModuleHandle) {
        fn remove_dependents<T: BufferElem>(host: &mut Host, module: &ModuleInternals) {
            for out_port in T::get_buffers_out(&module.buf_out).iter() {
                for dep_handle in out_port.dependents.iter() {
                    host.set_buffer_in(dep_handle.clone(), BufferInPort::default());
                }
            }
        }

        fn remove_dependencies<T: BufferElem>(
            host: &mut Host,
            module: &ModuleInternals,
            module_handle: &ModuleHandle,
        ) {
            for (port_idx, in_port) in T::get_buffers_in(&module.buf_in).iter().enumerate() {
                match in_port {
                    BufferInPort::OutBuffer(_) => {
                        host.set_buffer_in(
                            ModuleBufferInHandle {
                                module_handle: module_handle.clone(),
                                buf_handle: BufferInHandle {
                                    idx: port_idx,
                                    ..Default::default()
                                },
                            },
                            BufferInPort::<T>::default(),
                        );
                    }
                    BufferInPort::Constant(_) => {}
                }
            }
        }

        if *handle != self.output_handle {
            let module = self.modules.remove(&handle.idx).unwrap();
            remove_dependents::<f32>(self, &module);
            remove_dependencies::<f32>(self, &module, &handle);
            remove_dependents::<MidiEvents>(self, &module);
            remove_dependencies::<MidiEvents>(self, &module, &handle);
        }
    }

    // pub fn update_module<T: Module + ModuleTypes>(
    //     &mut self,
    //     handle: &ModuleHandle,
    //     settings: T::Settings,
    // ) {
    //     if let Some(module) = self.modules.get_mut(&handle.idx) {
    //         assert!(module.module.as_ref().type_id() == TypeId::of::<T>());
    //         let descriptor = T::init(settings);
    //         module.module = descriptor.initial_data;
    //         // module.buf_in.set_num_buffers_all(&descriptor.buf_in);
    //         // module.buf_out.set_num_buffers_all(&descriptor.buf_out);
    //     }
    // }

    pub fn process(&mut self) -> ! {
        let (_stream, stream_handle) = rodio::OutputStream::try_default().unwrap();
        stream_handle
            .play_raw(self.output.clone().stoppable())
            .unwrap();

        loop {
            for module in self.modules.values_mut() {
                *module.buf_in.num_finished_dependencies.get_mut() = 0;
            }

            let zero_dependency_mods = self
                .modules
                .iter()
                .filter_map(|(&idx, module)| {
                    if module.buf_in.num_dependencies == 0 {
                        Some(ModuleHandle { idx })
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();

            for handle in zero_dependency_mods {
                unsafe { self.process_module(handle) };
            }
        }
    }

    unsafe fn process_module(&mut self, handle: ModuleHandle) {
        fn get_linked_ports<T: BufferElem>(
            host: &Host,
            module: &ModuleInternals,
        ) -> Vec<*const Buffer<T>> {
            T::get_buffers_in(&module.buf_in)
                .iter()
                .map(|port| match port {
                    BufferInPort::OutBuffer(handle) => {
                        let buf_out = &host.modules.get(&handle.module_handle.idx).unwrap().buf_out;
                        &T::get_buffers_out(buf_out)[handle.buf_handle.idx].buffer as *const _
                    }
                    BufferInPort::Constant(buf) => buf,
                })
                .collect::<Vec<_>>()
        }

        fn get_out_buffers<T: BufferElem>(module: &mut ModuleInternals) -> Vec<*mut Buffer<T>> {
            T::get_buffers_out_mut(&mut module.buf_out)
                .iter_mut()
                .map(|buf| &mut buf.buffer as *mut _)
                .collect()
        }

        fn get_dependents<T: BufferElem>(
            module: &ModuleInternals,
        ) -> impl Iterator<Item = ModuleHandle> + '_ {
            T::get_buffers_out(&module.buf_out)
                .iter()
                .flat_map(|buf| &buf.dependents)
                .map(|h| h.module_handle.clone())
        }

        let module = self.modules.get_mut(&handle.idx).unwrap() as *mut ModuleInternals;

        let module_ref = &*module;

        if module_ref
            .buf_in
            .num_finished_dependencies
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
            < module_ref.buf_in.num_dependencies
        {
            return;
        }

        let buf_in = ModuleBuffersIn {
            buf_signal: get_linked_ports(&self, module_ref),
            buf_midi: get_linked_ports(&self, module_ref),
        };

        let module_mut = &mut *module;
        let mut buf_out = ModuleBuffersOut {
            buf_signal: get_out_buffers(module_mut),
            buf_midi: get_out_buffers(module_mut),
        };
        module_mut.module.fill_buffers(&buf_in, &mut buf_out);

        for dependent in
            get_dependents::<f32>(module_mut).chain(get_dependents::<MidiEvents>(module_mut))
        {
            self.process_module(dependent);
        }
    }
}
